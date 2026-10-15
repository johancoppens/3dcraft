//! The shell: viewport, feature tree and status bar over one engine
//! session. Thin: it reads the scene and draws it, and owns nothing but
//! view state. It never edits a document; that takes a command
//! (`Session::execute`), from whoever drives the session.

use crate::backend::{Backend, BodyRenderer, RendererChoice, RendererReport};
#[cfg(not(target_arch = "wasm32"))]
use crate::direct::{Direct, DirectFailure};
use crate::frames::{Mode, Scheduler, Ticket};
use crate::state::VertexRef;
use crate::state::{
    CameraState, EdgeRef, FaceRef, Picked, PickedEntity, PickedKind, ProjectionKind, SectionState,
    UiState, Work,
};
use crate::theme::{Tokens, color, color_alpha};
use crate::view_cube;
use crate::viewport::{self, Composed, MAX_SIDE, MIN_SIDE, Overlay, Quality};
use craft3d_engine::{
    DocumentStatus, FeatureState, PickKind, Recoverable, RecoveryOverview, Scene, SceneNear,
    ScenePointed, SceneQuality, SectionPlane, Session,
};
use craft3d_math::Bbox3;
use craft3d_render::{Highlight, Section};
use std::sync::Arc;

mod tabs;
pub use tabs::{
    CLOSE_MARK, CloseAnswer, DocAction, TabPart, UNSAVED_MARK, close_shortcut, doc_tabs,
    next_shortcut, previous_shortcut, tab_name,
};

/// The viewport's size before the window has laid it out, in pixels: what
/// `ui.screenshot` renders in a host without a window.
pub const DEFAULT_VIEWPORT: [u32; 2] = [960, 640];

/// While the camera moves the image is rendered this much smaller per side
/// and without supersampling, so orbiting stays fluid on the CPU renderer.
/// (None of the rules below apply to the direct GPU path of the window,
/// [`crate::direct`]: there every frame is the viewport's full size, with
/// multisampling, and nothing is read back.)
pub const MOVING_DIVISOR: u32 = 2;
/// Supersampling per axis of the image at rest, when the budget allows.
pub const REST_SSAA: u32 = 2;
/// Most samples (pixels times supersampling squared) in one image. The
/// CPU renderer's time is nearly all per sample (measured: about 30 ns per
/// sample, whatever the triangle count up to a few hundred thousand), so
/// these bound a frame to roughly 120 ms at rest and 20 ms while moving on
/// the developer's machine. A viewport with more pixels than the budget
/// gets a smaller image of the same shape, stretched over it.
pub const REST_SAMPLES: u64 = 4_000_000;
pub const MOVING_SAMPLES: u64 = 600_000;
/// The budgets of the GPU renderer are in pixels, not samples: drawing
/// is cheap there (under a millisecond for the draw itself), and what
/// bounds a frame is reading the picture back, 12 bytes per pixel out of
/// memory the driver maps uncached (measured on an Intel iGPU over
/// Vulkan: 50 ns per pixel for a small image and more for a large one,
/// so about 50 ms for a 1020 × 777 image and over 600 ms for 2560 × 1400),
/// and putting it in a texture. So the GPU image is supersampled at rest
/// within fewer pixels than the CPU's budget in samples allows, and while
/// moving it is [`MOVING_DIVISOR`] times smaller per side, as on the CPU,
/// within fewer pixels still. On a light model the CPU renderer is as
/// fast as this; on a heavy one the GPU wins. Doing without the readback
/// (the overlay's depth test on the GPU) is what would make it fast.
pub const GPU_REST_PIXELS: u64 = 2_000_000;
pub const GPU_MOVING_PIXELS: u64 = 400_000;

/// The product's name, as the window's title ends with it.
pub const PRODUCT: &str = "3DCraft";

/// What the File menu and its shortcuts ask for. The shell only asks: the
/// host runs the command, after showing its file dialog where one is
/// needed (`file.new`, `file.open`, `file.save`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    New,
    Open,
    /// To the document's own file; like `SaveAs` when it has none.
    Save,
    SaveAs,
}

impl FileAction {
    /// In menu order.
    pub const ALL: [FileAction; 4] = [
        FileAction::New,
        FileAction::Open,
        FileAction::Save,
        FileAction::SaveAs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FileAction::New => "New",
            FileAction::Open => "Open…",
            FileAction::Save => "Save",
            FileAction::SaveAs => "Save As…",
        }
    }

    /// Ctrl (⌘ on a Mac) with N, O, S, and Shift+S.
    pub fn shortcut(self) -> egui::KeyboardShortcut {
        let command = egui::Modifiers::COMMAND;
        let (modifiers, key) = match self {
            FileAction::New => (command, egui::Key::N),
            FileAction::Open => (command, egui::Key::O),
            FileAction::Save => (command, egui::Key::S),
            FileAction::SaveAs => (command | egui::Modifiers::SHIFT, egui::Key::S),
        };
        egui::KeyboardShortcut::new(modifiers, key)
    }

    /// Saving needs a document to save.
    pub fn needs_document(self) -> bool {
        matches!(self, FileAction::Save | FileAction::SaveAs)
    }
}

/// The window's title for a document: its file's name (`untitled` when it
/// has none), a `*` when it has changes that are not on disk, and the
/// product's name. Just the product's name without a document.
pub fn window_title(status: Option<&DocumentStatus>) -> String {
    let Some(status) = status else {
        return PRODUCT.to_string();
    };
    // The name, not the folders it is in. Either separator: a file saved
    // on another system keeps its path as it was written.
    let name = status
        .path
        .as_deref()
        .and_then(|path| path.rsplit(['/', '\\']).find(|part| !part.is_empty()))
        .unwrap_or("untitled");
    // No control characters in a title bar.
    let name: String = name.chars().filter(|c| !c.is_control()).take(120).collect();
    let dirty = if status.dirty { "*" } else { "" };
    format!("{name}{dirty} – {PRODUCT}")
}

/// One row of the feature tree, as it is shown.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeRow {
    /// `F3`.
    pub id: String,
    pub name: String,
    /// `block`, `sketch`, …
    pub kind: &'static str,
    pub state: FeatureState,
    /// `ok`, `failed`, `suppressed`, `rolled back`; for a sketch that
    /// solved, how far it is constrained.
    pub status: String,
    /// Why the feature failed.
    pub error: Option<String>,
    /// When it failed because something it is made from failed: the
    /// feature at the start of that chain, the one to fix, as its id
    /// (`F3`) and its name.
    pub caused_by: Option<(String, String)>,
    pub selected: bool,
}

/// The marks of the feature tree, by state: built, failed, suppressed,
/// rolled back. All four are in egui's default fonts (a test checks: the
/// round dot U+25CF is not, and was drawn as a box).
pub const TREE_MARKS: [char; 4] = ['✔', '⚠', '⏸', '○'];

/// The mark a row of the feature tree starts with, which says its state
/// at a glance; its colour says it too.
pub fn tree_mark(state: FeatureState) -> String {
    let [ok, failed, suppressed, rolled_back] = TREE_MARKS;
    match state {
        FeatureState::Ok => ok,
        FeatureState::Failed => failed,
        FeatureState::Suppressed => suppressed,
        FeatureState::RolledBack => rolled_back,
    }
    .to_string()
}

/// What the status bar says about the last command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandReport {
    /// The command id, or the protocol method.
    pub label: String,
    pub ok: bool,
    /// A short form of the result, or the error.
    pub text: String,
}

/// How the image in the viewport was last rendered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderStats {
    pub width: u32,
    pub height: u32,
    pub ssaa: u32,
    pub triangles: usize,
    /// How long rendering it took, on whichever thread did it. `None`
    /// where there is no clock (a browser).
    pub milliseconds: Option<f64>,
    /// False for the quick preview shown while the camera moves and until
    /// the sharp image is ready.
    pub sharp: bool,
    /// Which renderer shaded the bodies.
    pub renderer: Backend,
    /// Samples per pixel: 1 without multisampling.
    pub msaa: u32,
}

/// How long the camera takes to turn to the view a click on the view
/// cube asked for.
const TURN_SECONDS: f64 = 0.25;

/// A turn of the camera in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Turn {
    from: CameraState,
    to: CameraState,
    /// egui's time when it began.
    started: f64,
    /// The camera it set last: another one means something else moved it.
    last: CameraState,
}

/// What the image in the viewport is made from: another image is made
/// only when one of these changes.
#[derive(Clone, Debug, PartialEq)]
struct FrameKey {
    revision: Option<u64>,
    /// The tolerance and the angle the scene was sampled with: another
    /// display quality is another mesh of the same revision.
    tolerance: Option<(f64, Option<f64>)>,
    camera: CameraState,
    /// The viewport, in pixels.
    size: [u32; 2],
    /// The backend's largest texture side.
    largest: usize,
    selected: Option<String>,
    /// The faces shown highlighted.
    highlight: Highlight,
    /// The edge shown highlighted.
    highlight_edge: Option<EdgeRef>,
    /// The vertex shown highlighted.
    highlight_vertex: Option<VertexRef>,
    /// How opaque the bodies are.
    opacity: f64,
    /// The plane the picture is cut open with.
    section: Option<Section>,
    /// The datums are drawn.
    datums: bool,
    /// The sketches are drawn.
    sketches: bool,
    tokens: Tokens,
}

/// The section of the viewport as it lies in the scene that is shown:
/// resolved once per scene and per state, not per frame.
struct SectionCache {
    revision: u64,
    tolerance: (f64, Option<f64>),
    state: SectionState,
    cap: crate::theme::Rgb,
    plane: SectionPlane,
    /// The offsets between which the plane passes through the bodies.
    range: Option<[f64; 2]>,
    cut: Option<viewport::SectionCut>,
}

/// One image to make.
struct RenderJob {
    scene: Option<Arc<Scene>>,
    camera: CameraState,
    quality: Quality,
    tokens: Tokens,
    selected: Option<String>,
    highlight: Highlight,
    marks: viewport::Marks,
    opacity: f64,
    /// Asked to repaint when the image is ready.
    #[cfg(not(target_arch = "wasm32"))]
    wake: Option<egui::Context>,
}

/// An image that was made.
struct Rendered {
    composed: Composed,
    quality: Quality,
    milliseconds: Option<f64>,
    /// The renderer that made it, as it is after making it.
    report: RendererReport,
}

impl RenderJob {
    fn render(&self, renderer: &mut BodyRenderer) -> Result<Rendered, String> {
        let elapsed = stopwatch();
        renderer.set_highlight(self.highlight);
        renderer.set_marks(self.marks.clone());
        renderer.set_opacity(self.opacity);
        let composed = viewport::compose_with(
            renderer,
            self.scene.as_deref(),
            &self.camera,
            self.quality,
            &self.tokens,
            self.selected.as_deref(),
        )?;
        Ok(Rendered {
            composed,
            quality: self.quality,
            milliseconds: elapsed(),
            report: renderer.report().clone(),
        })
    }
}

/// A sharp image that arrived, or why it did not.
type Arrived = (Ticket, Result<Rendered, String>);

#[cfg(not(target_arch = "wasm32"))]
type RenderWorker = crate::frames::Worker<RenderJob, Rendered>;

/// What a button of the recovery bar asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Open this recoverable document (`recovery.open`).
    Open(String),
    /// Remove this one's recovery file (`recovery.discard`).
    Discard(String),
    /// Remove all of them.
    DiscardAll,
    /// Hide the bar and leave everything where it is.
    Later,
}

/// Most recoverable documents the bar lists; `recovery.list` has them all.
pub const RECOVERY_ROWS: usize = 6;

/// What the status bar says about autosave: the text, and whether it is a
/// problem. `None` when there is nothing to say. `now` is the time in
/// seconds since the Unix epoch, when the platform has a clock.
pub fn autosave_text(overview: &RecoveryOverview, now: Option<u64>) -> Option<(String, bool)> {
    if let Some(problem) = &overview.problem {
        return Some((format!("autosave failed: {problem}"), true));
    }
    let at = overview.last_autosave?;
    let Some(age) = now.map(|now| now.saturating_sub(at)) else {
        return Some(("autosaved".to_string(), false));
    };
    let text = match age {
        0..60 => "autosaved just now".to_string(),
        60..3600 => format!("autosaved {} min ago", age / 60),
        _ => format!("autosaved {} h ago", age / 3600),
    };
    Some((text, false))
}

/// One line about a recoverable document, after its name.
fn recoverable_text(found: &Recoverable, now: Option<u64>) -> String {
    let mut parts = Vec::new();
    if let Some(features) = found.features {
        parts.push(format!("{features} features"));
    }
    if let (Some(at), Some(now)) = (found.time, now) {
        let age = now.saturating_sub(at);
        parts.push(match age {
            0..60 => "autosaved just now".to_string(),
            60..3600 => format!("autosaved {} min ago", age / 60),
            3600..172_800 => format!("autosaved {} h ago", age / 3600),
            _ => format!("autosaved {} days ago", age / 86_400),
        });
    }
    if let Some(path) = &found.path {
        parts.push(path.clone());
    }
    if let Some(problem) = &found.problem {
        parts.push(problem.clone());
    }
    parts.join(" · ")
}

#[cfg(not(target_arch = "wasm32"))]
fn epoch_now() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs())
}

#[cfg(target_arch = "wasm32")]
fn epoch_now() -> Option<u64> {
    None
}

pub struct Shell {
    pub state: UiState,
    pub tokens: Tokens,
    pub last_command: Option<CommandReport>,
    /// Autosave and what can be recovered, as of the last `sync`.
    recovery: RecoveryOverview,
    /// Where the buttons of the recovery bar were drawn in the last frame.
    recovery_hits: Vec<(RecoveryAction, egui::Rect)>,
    /// The discard that was clicked once and waits for its second click.
    discard_armed: Option<RecoveryAction>,
    /// How many picks were made in the viewport ([`Picked::serial`]).
    picks: u64,
    /// Viewport size in pixels, and pixels per point.
    size: [u32; 2],
    pixels_per_point: f32,
    /// Where the viewport is in the window, in points.
    rect: Option<egui::Rect>,
    texture: Option<egui::TextureHandle>,
    /// What is drawn over the texture, in the pixels of its image.
    overlay: Overlay,
    /// The size of that image.
    overlay_size: [u32; 2],
    /// Which image is on screen and which is being made.
    scheduler: Scheduler<FrameKey>,
    /// The thread sharp images are made on; `None` makes them here.
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<RenderWorker>,
    /// Sharp images that are ready and not on screen yet.
    arrived: Vec<Arrived>,
    /// The renderer of this thread: previews, and sharp images when there
    /// is no render thread.
    renderer: BodyRenderer,
    /// The renderer that made the image on screen, as it reported itself.
    shown_report: Option<RendererReport>,
    /// The GPU on the window's own device, when the window lent it: the
    /// path that draws every frame in full and reads nothing back.
    #[cfg(not(target_arch = "wasm32"))]
    direct: Option<Direct>,
    /// Why the direct path is not used although the GPU was wanted.
    direct_fallback: Option<String>,
    /// True when the picture on screen is the direct path's texture.
    direct_shown: bool,
    stats: Option<RenderStats>,
    /// How many times the image was rendered.
    renders: u64,
    /// The camera when the drag in progress began.
    drag_from: Option<CameraState>,
    /// The camera when a drag on the view cube began.
    cube_drag_from: Option<CameraState>,
    /// Where the view cube was drawn.
    view_cube_rect: Option<egui::Rect>,
    /// The turn a click on the view cube started, while it lasts.
    turn: Option<Turn>,
    /// Where the see-through slider was drawn.
    transparency_rect: Option<egui::Rect>,
    /// Where the section's controls were drawn: `toggle` and, while the
    /// section is on, `x`, `y`, `z`, `offset` and `flip`.
    section_hits: Vec<(&'static str, egui::Rect)>,
    /// Where the Sketches and Datums toggles were drawn.
    show_hits: Vec<(&'static str, egui::Rect)>,
    /// Where the last frame drew each "caused by" link of the tree: the
    /// row's feature id, the id of the feature it names, and the link.
    cause_hits: Vec<(String, String, egui::Rect)>,
    /// The section as it lies in the scene, while it is on.
    section_cache: Option<SectionCache>,
    /// Why the section was switched off: its plane was lost.
    section_lost: Option<String>,
    /// The box the camera was last fitted to automatically.
    framed: Bbox3,
    themed: bool,
    quit: bool,
    rows: Vec<TreeRow>,
    /// What the File menu or a shortcut asked for, until the host takes it.
    file_action: Option<FileAction>,
    /// Why the scene could not be made.
    problem: Option<String>,
    /// Why the image could not be made; cleared by the next image.
    render_problem: Option<String>,
    /// The scene and the document's status as of the last [`Self::sync`]:
    /// what is drawn while the session is away doing work.
    shown_scene: Option<Arc<Scene>>,
    shown_status: Option<DocumentStatus>,
    /// The work being done on the host's engine thread, if any.
    work: Option<Work>,
    /// The open documents as of the last sync: the row of tabs.
    tabs: Vec<crate::state::DocTab>,
    /// The question before closing a document, while it is asked.
    closing: Option<crate::state::Closing>,
    /// What a tab, the File menu or a shortcut asked for, until the host
    /// takes it.
    doc_action: Option<DocAction>,
    /// Where the last frame drew the tabs and their close buttons.
    tab_hits: Vec<(TabPart, egui::Rect)>,
    /// Where the last frame drew the question's buttons.
    closing_hits: Vec<(CloseAnswer, egui::Rect)>,
    /// The tab that was last brought into view, by its place in the row.
    tab_scrolled: Option<usize>,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

fn status_text(state: FeatureState, is_sketch: bool, dof: Option<usize>) -> String {
    match (state, is_sketch, dof) {
        (FeatureState::Ok, true, Some(0)) => "fully constrained".to_string(),
        (FeatureState::Ok, true, Some(dof)) => format!("{dof} DOF"),
        (FeatureState::Ok, _, _) => "ok".to_string(),
        (FeatureState::Failed, _, _) => "failed".to_string(),
        (FeatureState::Suppressed, _, _) => "suppressed".to_string(),
        (FeatureState::RolledBack, _, _) => "rolled back".to_string(),
    }
}

/// A clock for timing a render, where the platform has one.
#[cfg(not(target_arch = "wasm32"))]
fn stopwatch() -> impl FnOnce() -> Option<f64> {
    let start = std::time::Instant::now();
    move || Some(start.elapsed().as_secs_f64() * 1000.0)
}

#[cfg(target_arch = "wasm32")]
fn stopwatch() -> impl FnOnce() -> Option<f64> {
    || None
}

impl Shell {
    /// A shell that renders on the CPU, where it draws: what tests and a
    /// host without threads use.
    pub fn new() -> Self {
        Self::with_renderer(RendererChoice::Cpu)
    }

    /// A shell that renders with the renderer asked for (the GPU when
    /// there is one, for [`RendererChoice::Auto`]), where it draws.
    pub fn with_renderer(choice: RendererChoice) -> Self {
        Self {
            state: UiState::default(),
            tokens: Tokens::default(),
            last_command: None,
            recovery: RecoveryOverview::default(),
            recovery_hits: Vec::new(),
            discard_armed: None,
            picks: 0,
            size: DEFAULT_VIEWPORT,
            pixels_per_point: 1.0,
            rect: None,
            texture: None,
            overlay: Overlay::default(),
            overlay_size: DEFAULT_VIEWPORT,
            scheduler: Scheduler::new(Mode::Inline),
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
            arrived: Vec::new(),
            renderer: BodyRenderer::new(choice),
            shown_report: None,
            #[cfg(not(target_arch = "wasm32"))]
            direct: None,
            direct_fallback: None,
            direct_shown: false,
            stats: None,
            renders: 0,
            drag_from: None,
            cube_drag_from: None,
            view_cube_rect: None,
            turn: None,
            transparency_rect: None,
            section_hits: Vec::new(),
            show_hits: Vec::new(),
            cause_hits: Vec::new(),
            section_cache: None,
            section_lost: None,
            framed: Bbox3::EMPTY,
            themed: false,
            quit: false,
            rows: Vec::new(),
            file_action: None,
            problem: None,
            render_problem: None,
            shown_scene: None,
            shown_status: None,
            work: None,
            tabs: Vec::new(),
            closing: None,
            doc_action: None,
            tab_hits: Vec::new(),
            closing_hits: Vec::new(),
            tab_scrolled: None,
        }
    }

    /// A shell that has its sharp images made on a thread of its own, so
    /// the thread that draws the window only ever makes quick previews.
    /// [`Self::new`] makes every image where it is drawn, which is what a
    /// host without threads needs and what makes a frame's outcome
    /// independent of timing. Renders on the CPU; a window uses
    /// [`Self::threaded_with`].
    ///
    /// When the thread cannot be started this is the same as `new`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn threaded() -> Self {
        Self::threaded_with(RendererChoice::Cpu)
    }

    /// [`Self::threaded`] with the renderer asked for on both threads:
    /// each has a renderer of its own (two GPU devices, for
    /// [`RendererChoice::Auto`] on a machine with an adapter), made on the
    /// thread that uses it. What a window uses.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn threaded_with(choice: RendererChoice) -> Self {
        let mut shell = Self::with_renderer(choice);
        // The thread's renderer is made on the thread, at its first job.
        let mut renderer: Option<BodyRenderer> = None;
        let worker = RenderWorker::start(
            move |job: &RenderJob| {
                job.render(renderer.get_or_insert_with(|| BodyRenderer::new(choice)))
            },
            |job: &RenderJob| {
                if let Some(ctx) = &job.wake {
                    ctx.request_repaint();
                }
            },
        );
        if let Ok(worker) = worker {
            shell.worker = Some(worker);
            shell.scheduler.set_mode(Mode::Background);
        }
        shell
    }

    /// [`Self::threaded_with`] for a window that lends the shell its own
    /// wgpu device (`state`): with [`RendererChoice::Auto`] the viewport
    /// is then drawn on that device, directly into the texture that is
    /// shown ([`crate::direct`]), and the CPU renderer stands by for the
    /// frames that path cannot make. When the direct path cannot be set
    /// up, or without a `state`, this is [`Self::threaded_with`], with the
    /// reason kept for [`Self::renderer_report`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn threaded_with_gpu(
        choice: RendererChoice,
        state: Option<&egui_wgpu::RenderState>,
    ) -> Self {
        if choice == RendererChoice::Auto
            && let Some(state) = state
        {
            match Direct::new(state) {
                Ok(direct) => {
                    let mut shell = Self::threaded_with(RendererChoice::Cpu);
                    shell.direct = Some(direct);
                    return shell;
                }
                Err(why) => {
                    let mut shell = Self::threaded_with(choice);
                    shell.direct_fallback = Some(why);
                    return shell;
                }
            }
        }
        Self::threaded_with(choice)
    }

    /// Draws the viewport on the device of `state` from now on, directly
    /// into the texture that is shown ([`crate::direct`]), with this
    /// shell's renderer standing by for what that path cannot make. For a
    /// host that drives the shell headlessly with a device of its own (a
    /// test); the window uses [`Self::threaded_with_gpu`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn attach_gpu(&mut self, state: &egui_wgpu::RenderState) -> Result<(), String> {
        let direct = Direct::new(state)?;
        self.direct = Some(direct);
        self.direct_fallback = None;
        Ok(())
    }

    /// As on a device that cannot draw see-through bodies
    /// (`craft3d_gpu::GpuError::SeeThrough`): from now on a see-through
    /// frame is drawn by the CPU renderer, on this thread. For tests of
    /// that fallback (a render thread's renderer is not reached).
    pub fn without_gpu_see_through(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(direct) = &mut self.direct {
            direct.without_see_through();
        }
        self.renderer.without_gpu_see_through();
    }

    /// As on a device that cannot draw a cut-open picture: from now on a
    /// frame with a section is the CPU renderer's. For tests of that
    /// fallback.
    pub fn without_gpu_section(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(direct) = &mut self.direct {
            direct.without_section();
        }
        self.renderer.without_gpu_section();
    }

    /// There are no threads here: the same as [`Self::new`].
    #[cfg(target_arch = "wasm32")]
    pub fn threaded() -> Self {
        Self::new()
    }

    /// There are no threads here: the same as [`Self::with_renderer`].
    #[cfg(target_arch = "wasm32")]
    pub fn threaded_with(choice: RendererChoice) -> Self {
        Self::with_renderer(choice)
    }

    /// What the viewport renders with: the GPU on the window's device when
    /// it draws directly; else the renderer that made the image on screen,
    /// or, before there is one, this thread's.
    pub fn renderer_report(&self) -> RendererReport {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(direct) = &self.direct {
            return direct.report();
        }
        let mut report = self
            .shown_report
            .clone()
            .unwrap_or_else(|| self.renderer.report().clone());
        if report.fallback.is_none() {
            report.fallback.clone_from(&self.direct_fallback);
        }
        report
    }

    /// The picture the direct path shows, read back from the device: for
    /// tests, which compare it with the CPU reference renderer's. `None`
    /// when the viewport is not drawn directly or nothing was drawn yet.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn direct_picture(&self) -> Option<Result<crate::direct::ReadBack, String>> {
        self.direct
            .as_ref()
            .filter(|_| self.direct_shown)
            .map(Direct::read_rgb)
    }

    /// True when the viewport is drawn directly on the window's device.
    pub fn draws_directly(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.direct.is_some()
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }

    /// The quality the viewport asks the engine for: the display rule
    /// with the state's `quality` ([`crate::state::DisplayQuality`]). The
    /// scene the shell shows, and `ui.screenshot` renders, is
    /// `session.scene_with(shell.scene_quality())`.
    pub fn scene_quality(&self) -> SceneQuality {
        self.state.quality.scene_quality()
    }

    /// True when sharp images are made on a thread of their own.
    pub fn renders_in_background(&self) -> bool {
        self.scheduler.mode() == Mode::Background
    }

    /// Waits until the sharp image that is being made has been made, for
    /// at most `timeout`. The next frame puts it on screen (if it is still
    /// the image of what that frame shows). True when nothing is being
    /// made any more; false when the time ran out.
    ///
    /// For hosts that need the finished picture now (a recording, a test):
    /// the window itself never waits.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_for_sharp(&mut self, timeout: std::time::Duration) -> bool {
        let Some(ticket) = self.scheduler.pending() else {
            return true;
        };
        if self.arrived.iter().any(|(arrived, _)| *arrived == ticket) {
            return true;
        }
        let Some(worker) = &self.worker else {
            return false;
        };
        let deadline = std::time::Instant::now().checked_add(timeout);
        loop {
            let left = deadline.map_or(timeout, |deadline| {
                deadline.saturating_duration_since(std::time::Instant::now())
            });
            let Some(done) = worker.wait(left) else {
                return false;
            };
            let found = done.ticket == ticket;
            self.arrived.push((done.ticket, done.result));
            if found {
                return true;
            }
            if left.is_zero() {
                return false;
            }
        }
    }

    /// Nothing is made elsewhere here, so there is nothing to wait for.
    #[cfg(target_arch = "wasm32")]
    pub fn wait_for_sharp(&mut self, _timeout: std::time::Duration) -> bool {
        true
    }

    /// The viewport's size in pixels.
    pub fn viewport_size(&self) -> [u32; 2] {
        self.size
    }

    pub fn pixels_per_point(&self) -> f32 {
        self.pixels_per_point
    }

    /// Where the viewport was laid out in the last frame, in points.
    pub fn viewport_rect(&self) -> Option<egui::Rect> {
        self.rect
    }

    /// The rows of the feature tree as of the last [`Self::sync`].
    pub fn tree_rows(&self) -> &[TreeRow] {
        &self.rows
    }

    /// What is drawn over the viewport's texture, in the pixels of the
    /// image that texture holds, and that image's size: for a host that
    /// inks the picture itself (the `snapshot` example). On the direct
    /// path the lines are in the texture already and this is what the
    /// last frame of another path drew, if any.
    pub fn overlay(&self) -> (&Overlay, [u32; 2]) {
        (&self.overlay, self.overlay_size)
    }

    pub fn render_stats(&self) -> Option<RenderStats> {
        self.stats
    }

    /// How many times the viewport image has been rendered.
    pub fn render_count(&self) -> u64 {
        self.renders
    }

    /// Why there is no picture, or not the right one, when that is so.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref().or(self.render_problem.as_deref())
    }

    /// Says what the host's engine thread is doing (`None`: nothing). The
    /// window shows it over the viewport, `ui.get` under `work`.
    pub fn set_work(&mut self, work: Option<Work>) {
        self.work = work;
    }

    /// The work in progress, as last set.
    pub fn work(&self) -> Option<&Work> {
        self.work.as_ref()
    }

    /// The document's status as of the last [`Self::sync`]: what the
    /// window shows while the session is away.
    pub fn shown_status(&self) -> Option<&DocumentStatus> {
        self.shown_status.as_ref()
    }

    /// The scene as of the last [`Self::sync`].
    pub fn shown_scene_arc(&self) -> Option<Arc<Scene>> {
        self.shown_scene.clone()
    }

    /// True once something asked the application to close.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    pub fn request_quit(&mut self) {
        self.quit = true;
    }

    /// What the File menu or a shortcut asked for since the last call.
    /// The host takes it and does it: the shell has no file dialog and
    /// does not touch files.
    pub fn take_file_action(&mut self) -> Option<FileAction> {
        self.file_action.take()
    }

    /// Asks for a file action, as the menu does.
    pub fn request_file_action(&mut self, action: FileAction) {
        self.file_action = Some(action);
    }

    /// The faces the viewport shows highlighted, as the renderers take
    /// them.
    pub fn face_highlight(&self, scene: Option<&Scene>) -> Highlight {
        self.state
            .highlight
            .as_ref()
            .map_or(Highlight::NONE, |face| {
                viewport::face_highlight(scene, face.feature.as_deref(), &face.face, &self.tokens)
            })
    }

    /// What the viewport draws over the picture to point at something
    /// (the highlighted edge and the highlighted vertex), and the section
    /// the picture is cut open with.
    pub fn marks(&self, scene: Option<&Scene>) -> viewport::Marks {
        marks_of(
            scene,
            self.state.highlight_edge.as_ref(),
            self.state.highlight_vertex.as_ref(),
            self.section_cut(),
            self.state.show.datums,
            self.state.show.sketches,
        )
    }

    /// The plane the viewport's picture is cut open with, as the
    /// renderers take it: `None` when the section is off, and before the
    /// first [`Self::sync`] with a document.
    pub fn section(&self) -> Option<Section> {
        self.section_cache
            .as_ref()
            .map(|cache| cache.plane.section(cache.cap))
    }

    /// The section made ready for the scene that is shown.
    fn section_cut(&self) -> Option<viewport::SectionCut> {
        self.section_cache
            .as_ref()
            .and_then(|cache| cache.cut.clone())
    }

    /// What `ui.get` says about the section: where the plane lies and the
    /// offsets between which it passes through the bodies, or why it was
    /// switched off; `None` when it is off and nothing was lost.
    pub fn section_report(&self) -> Option<serde_json::Value> {
        let vec3 = |v: craft3d_math::Vec3| serde_json::json!([v.x, v.y, v.z]);
        match (&self.section_cache, &self.section_lost) {
            (Some(cache), _) => Some(serde_json::json!({
                "point": vec3(cache.plane.origin),
                "normal": vec3(cache.plane.normal),
                "range": cache.range,
                "lost": null,
            })),
            (None, Some(why)) => Some(serde_json::json!({ "lost": why })),
            (None, None) => None,
        }
    }

    /// Why the section was switched off, when its plane was lost.
    pub fn section_lost(&self) -> Option<&str> {
        self.section_lost.as_deref()
    }

    /// Forgets why the section was switched off: it was set again.
    pub fn clear_section_lost(&mut self) {
        self.section_lost = None;
    }

    /// Where the section's controls are in the window, in points, once
    /// the viewport was drawn: `toggle` and, while the section is on,
    /// `x`, `y`, `z`, `offset` (the slider) and `flip`.
    pub fn section_hits(&self) -> &[(&'static str, egui::Rect)] {
        &self.section_hits
    }

    /// Where the toggles of what is drawn are in the window, in points,
    /// once the viewport was drawn: `sketches` and `datums`.
    pub fn show_hits(&self) -> &[(&'static str, egui::Rect)] {
        &self.show_hits
    }

    /// Where the last frame drew the "caused by" links of the feature tree:
    /// (the failed feature's id, the id of the feature that caused it, the
    /// link's rectangle). A click on a link selects that feature.
    pub fn cause_hits(&self) -> &[(String, String, egui::Rect)] {
        &self.cause_hits
    }

    /// Brings the section up to date with the scene: where its plane lies
    /// now. A plane that no longer resolves (its face or datum is gone)
    /// switches the section off, with the reason kept for `ui.get`.
    fn resolve_section(&mut self, session: &mut Session, scene: &Scene) {
        if !self.state.section.on {
            self.section_cache = None;
            return;
        }
        let cap = self.tokens.section_cap;
        let current = self.section_cache.as_ref().is_some_and(|cache| {
            cache.revision == scene.revision
                && cache.tolerance == scene.sampling()
                && cache.state == self.state.section
                && cache.cap == cap
        });
        if current {
            return;
        }
        match session.section_plane(scene, &self.state.section.spec()) {
            Ok(plane) => {
                self.section_cache = Some(SectionCache {
                    revision: scene.revision,
                    tolerance: scene.sampling(),
                    state: self.state.section.clone(),
                    cap,
                    plane,
                    range: plane.range(&craft3d_render::section_bounds(&scene.mesh)),
                    cut: viewport::SectionCut::new(scene, plane.section(cap)),
                });
                self.section_lost = None;
            }
            Err(why) => {
                self.state.section.on = false;
                self.section_cache = None;
                self.section_lost = Some(why);
            }
        }
    }

    /// How opaque the viewport draws the bodies, 0.1 to 1: one minus
    /// [`UiState::transparency`].
    pub fn opacity(&self) -> f64 {
        let transparency = self.state.transparency;
        if transparency.is_finite() {
            1.0 - transparency.clamp(0.0, crate::state::MAX_TRANSPARENCY)
        } else {
            1.0
        }
    }

    /// Where the see-through slider is in the window, in points, once the
    /// viewport was drawn.
    pub fn transparency_rect(&self) -> Option<egui::Rect> {
        self.transparency_rect
    }

    /// How near to an edge a click counts as a click on it, in pixels of
    /// the viewport: the theme's `pick_radius` points.
    pub fn pick_radius(&self) -> f64 {
        let radius = f64::from(self.tokens.pick_radius) * f64::from(self.pixels_per_point);
        if radius.is_finite() {
            radius.clamp(0.0, craft3d_engine::MAX_PICK_RADIUS)
        } else {
            craft3d_engine::DEFAULT_PICK_RADIUS
        }
    }

    /// What the viewport shows at (`x`, `y`): pixels from the top-left
    /// corner of a picture of `size` (the viewport's own when `None`), as
    /// framed by the camera now. The answer is the engine's
    /// (`Scene::point_at`) on the mesh the viewport draws, so it is the
    /// face the picture has there, whichever renderer made the picture,
    /// and the edge within `radius` pixels of that picture, by the
    /// engine's rule (`None`: [`Self::pick_radius`], as many pixels of
    /// that picture as it is of the viewport).
    ///
    /// With `select` this is a click: the answer becomes
    /// [`UiState::picked`], what it points at the highlight (the edge, or
    /// else the face), and a pick on nothing clears the highlights.
    pub fn pick(
        &mut self,
        scene: Option<&Scene>,
        x: f64,
        y: f64,
        size: Option<[u32; 2]>,
        radius: Option<f64>,
        select: bool,
    ) -> Result<ScenePointed, String> {
        let [width, height] = size.unwrap_or(self.size);
        let radius = radius.unwrap_or_else(|| {
            // The same place on the model in a picture of another size.
            let stretch = f64::from(width) / f64::from(self.size[0].max(1));
            self.pick_radius() * stretch
        });
        // Of the picture as it is shown: cut open when the section is on.
        let section = self.section();
        let found = match scene {
            Some(scene) => scene.point_at_cut(
                &self.state.camera.to_render(),
                width,
                height,
                x,
                y,
                radius,
                section.as_ref(),
            )?,
            None => ScenePointed::default(),
        };
        if select && x.is_finite() && y.is_finite() {
            self.picks = self.picks.saturating_add(1);
            let hit = found.face.as_ref();
            let entity = |near: &SceneNear| PickedEntity {
                feature: near.feature.clone(),
                body: near.body.clone(),
                name: near.name.clone(),
                index: u64::try_from(near.index).unwrap_or(u64::MAX),
                faces: near.faces.clone(),
                point: [near.point.x, near.point.y, near.point.z],
            };
            // On the cap of the section: the body that is cut there and
            // the point of the plane, and no face.
            let cap = found.section.as_ref().filter(|_| hit.is_none());
            self.state.picked = Some(Picked {
                serial: self.picks,
                x,
                y,
                feature: hit
                    .map(|hit| hit.feature.clone())
                    .or_else(|| cap.and_then(|cap| cap.feature.clone())),
                name: hit
                    .map(|hit| hit.body.clone())
                    .or_else(|| cap.and_then(|cap| cap.body.clone())),
                face: hit.and_then(|hit| hit.face.clone()),
                face_index: hit.and_then(|hit| u64::try_from(hit.face_index).ok()),
                point: hit
                    .map(|hit| hit.point)
                    .or(cap.map(|cap| cap.point))
                    .map(|p| [p.x, p.y, p.z]),
                kind: found.kind().map(|kind| match kind {
                    PickKind::Face => PickedKind::Face,
                    PickKind::Edge => PickedKind::Edge,
                    PickKind::Vertex => PickedKind::Vertex,
                    PickKind::Section => PickedKind::Section,
                }),
                edge: found
                    .edge
                    .as_ref()
                    .filter(|_| found.vertex.is_none())
                    .map(entity),
                vertex: found.vertex.as_ref().map(entity),
            });
            // One thing is pointed at: the vertex, or else the edge, or
            // else the face.
            self.state.highlight_vertex = found
                .vertex
                .as_ref()
                .and_then(|near| named_vertex(&near.feature, near.name.as_deref(), &near.faces));
            self.state.highlight_edge = match found.kind() {
                Some(PickKind::Edge) => found.edge.as_ref().and_then(edge_ref),
                _ => None,
            };
            self.state.highlight = match found.kind() {
                Some(PickKind::Face) => hit.and_then(|hit| {
                    Some(FaceRef {
                        feature: Some(hit.feature.clone()),
                        face: hit.face.clone()?,
                    })
                }),
                _ => None,
            };
        }
        Ok(found)
    }

    fn aspect(&self) -> f64 {
        let [w, h] = self.size;
        f64::from(w.max(1)) / f64::from(h.max(1))
    }

    /// Brings the shell up to date with the session: the scene of the
    /// active document, the tree rows, the selection, and the camera when
    /// it follows the model. Called every frame and after every request.
    pub fn sync(&mut self, session: &mut Session) -> Option<Arc<Scene>> {
        let scene = self.sync_scene(session);
        self.sync_tabs(session);
        self.shown_status = session.document_status();
        self.shown_scene.clone_from(&scene);
        scene
    }

    fn sync_scene(&mut self, session: &mut Session) -> Option<Arc<Scene>> {
        self.recovery = session.recovery_overview();
        let scene = match session.scene_with(self.scene_quality()) {
            Ok(scene) => {
                self.problem = None;
                scene
            }
            Err(e) => {
                self.problem = Some(e.message().to_string());
                None
            }
        };
        let Some(scene) = scene else {
            self.rows.clear();
            self.state.selected_feature = None;
            self.state.highlight = None;
            self.state.highlight_edge = None;
            self.state.highlight_vertex = None;
            self.state.picked = None;
            self.framed = Bbox3::EMPTY;
            // The section stays as it is set: it is resolved again when
            // there is a document to cut.
            self.section_cache = None;
            return None;
        };
        self.resolve_section(session, &scene);
        if let Some(first) = scene.problems.first() {
            self.problem = Some(first.clone());
        }
        if self
            .state
            .selected_feature
            .as_ref()
            .is_some_and(|id| !scene.features.iter().any(|f| f.id == *id))
        {
            self.state.selected_feature = None;
        }
        // A face that is gone is not pointed at any more. A name is made
        // of where the face came from, so it survives the edits that keep
        // the face.
        if self.state.highlight.as_ref().is_some_and(|face| {
            scene
                .faces_named(face.feature.as_deref(), &face.face)
                .is_err()
        }) {
            self.state.highlight = None;
        }
        // An edge likewise: by its own name, or by the faces it lies
        // between.
        let edge_gone = |edge: &EdgeRef| {
            scene
                .edges_named(edge.feature.as_deref(), edge.edge.as_deref(), &edge.between)
                .is_err()
        };
        if self.state.highlight_edge.as_ref().is_some_and(edge_gone) {
            self.state.highlight_edge = None;
        }
        let vertex_gone = |vertex: &VertexRef| {
            scene
                .vertices_named(
                    vertex.feature.as_deref(),
                    vertex.vertex.as_deref(),
                    &vertex.corner,
                )
                .is_err()
        };
        if self
            .state
            .highlight_vertex
            .as_ref()
            .is_some_and(vertex_gone)
        {
            self.state.highlight_vertex = None;
        }
        if self.state.picked.as_ref().is_some_and(|picked| {
            picked
                .face
                .as_ref()
                .is_some_and(|face| scene.faces_named(picked.feature.as_deref(), face).is_err())
                || picked
                    .edge
                    .as_ref()
                    .is_some_and(|edge| picked_edge_ref(edge).is_none_or(|edge| edge_gone(&edge)))
        }) {
            self.state.picked = None;
        }
        let selected = self.state.selected_feature.as_deref();
        let name_of = |id: &str| {
            scene
                .features
                .iter()
                .find(|f| f.id == id)
                .map_or_else(|| id.to_string(), |f| f.name.clone())
        };
        self.rows = scene
            .features
            .iter()
            .map(|f| TreeRow {
                id: f.id.clone(),
                name: f.name.clone(),
                kind: f.kind,
                state: f.state,
                status: status_text(f.state, f.is_sketch, f.dof),
                error: f.error.clone(),
                caused_by: f.caused_by.as_ref().map(|id| (id.clone(), name_of(id))),
                selected: selected == Some(f.id.as_str()),
            })
            .collect();
        if self.state.camera.auto_fit && scene.bbox != self.framed {
            self.framed = scene.bbox;
            let aspect = self.aspect();
            self.state.camera.fit(&scene.bbox, aspect);
        }
        Some(scene)
    }

    /// Frames everything, and keeps following the model from now on.
    /// Returns false when there is nothing to frame.
    pub fn fit(&mut self, session: &mut Session) -> bool {
        let Some(scene) = self.sync(session) else {
            return false;
        };
        if scene.bbox.is_empty() {
            return false;
        }
        let aspect = self.aspect();
        self.state.camera.fit(&scene.bbox, aspect);
        self.state.camera.auto_fit = true;
        self.framed = scene.bbox;
        true
    }

    /// Draws the whole shell into `ui` (the window's root) for one frame.
    pub fn ui(&mut self, ui: &mut egui::Ui, session: &mut Session) {
        self.ui_with(ui, Some(session));
    }

    /// Draws the shell for one frame while its session is away, doing work
    /// on the host's engine thread ([`Self::set_work`]): the scene and the
    /// status of the last [`Self::sync`], the camera still turns, and the
    /// work in progress is shown over the viewport. What needs the session
    /// (the recovery bar's buttons) waits.
    pub fn ui_away(&mut self, ui: &mut egui::Ui) {
        self.ui_with(ui, None);
    }

    fn ui_with(&mut self, ui: &mut egui::Ui, mut session: Option<&mut Session>) {
        if !self.themed {
            self.tokens.apply(ui.ctx());
            self.themed = true;
        }
        let (scene, status) = match session.as_deref_mut() {
            Some(session) => (self.sync(session), session.document_status()),
            None => (self.shown_scene.clone(), self.shown_status.clone()),
        };
        self.file_shortcuts(ui, status.is_some());
        self.tab_shortcuts(ui);
        egui::Panel::top("craft3d-menu")
            .resizable(false)
            .show(ui, |ui| self.menu_bar(ui, status.is_some()));
        if self.file_action.is_some() || self.doc_action.is_some() {
            // The host does it between frames: have one come.
            ui.ctx().request_repaint();
        }
        if self.state.panels.recovery && !self.recovery.found.is_empty() {
            let clicked = egui::Panel::top("craft3d-recovery")
                .resizable(false)
                .show(ui, |ui| self.recovery_bar(ui))
                .inner;
            if let (Some(action), Some(session)) = (clicked, session) {
                self.recovery_act(session, action);
                ui.ctx().request_repaint();
            }
        } else {
            self.recovery_hits.clear();
            self.discard_armed = None;
        }
        if self.tabs.is_empty() {
            self.tab_hits.clear();
        } else {
            let tokens = self.tokens;
            egui::Panel::top("craft3d-tabs")
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(color(tokens.panel_fill))
                        .inner_margin(egui::Margin {
                            left: 4,
                            right: 4,
                            top: 3,
                            bottom: 0,
                        }),
                )
                .show(ui, |ui| self.tab_row(ui));
        }
        if self.state.panels.status {
            egui::Panel::bottom("craft3d-status")
                .resizable(false)
                .show(ui, |ui| self.status_bar(ui, status.as_ref()));
        }
        self.cause_hits.clear();
        if self.state.panels.tree {
            egui::Panel::left("craft3d-tree")
                .resizable(true)
                .default_size(260.0)
                .show(ui, |ui| self.tree(ui, status.is_some()));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, scene.as_ref()));
        self.closing_dialog(ui);
    }

    /// The File shortcuts, wherever the pointer is.
    fn file_shortcuts(&mut self, ui: &egui::Ui, has_document: bool) {
        // Shift+S before S: the longer shortcut would also match the
        // shorter one.
        for action in [
            FileAction::SaveAs,
            FileAction::Save,
            FileAction::Open,
            FileAction::New,
        ] {
            let pressed = ui.input_mut(|i| i.consume_shortcut(&action.shortcut()));
            if pressed && (has_document || !action.needs_document()) {
                self.file_action = Some(action);
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, has_document: bool) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                for action in FileAction::ALL {
                    let shortcut = ui.ctx().format_shortcut(&action.shortcut());
                    let button = egui::Button::new(action.label()).shortcut_text(shortcut);
                    let enabled = has_document || !action.needs_document();
                    if ui.add_enabled(enabled, button).clicked() {
                        self.file_action = Some(action);
                        ui.close();
                    }
                }
                self.menu_documents(ui);
            });
        });
    }

    /// Autosave and what the last look at the recovery directory found.
    pub fn recovery(&self) -> &RecoveryOverview {
        &self.recovery
    }

    /// The buttons of the recovery bar as the last frame drew them: what
    /// each asks for and where it is, in points. Empty when the bar is
    /// not shown.
    pub fn recovery_hits(&self) -> &[(RecoveryAction, egui::Rect)] {
        &self.recovery_hits
    }

    /// The discard that was asked for once and is done when asked again.
    pub fn discard_armed(&self) -> Option<&RecoveryAction> {
        self.discard_armed.as_ref()
    }

    /// Does what a button of the recovery bar asks for, through the
    /// engine's commands. Discarding cannot be undone, so the first
    /// request only arms it and the second one, for the same thing, does
    /// it; anything else in between disarms it.
    pub fn recovery_act(&mut self, session: &mut Session, action: RecoveryAction) {
        let (command, params) = match &action {
            RecoveryAction::Later => {
                self.state.panels.recovery = false;
                self.discard_armed = None;
                return;
            }
            RecoveryAction::Open(id) => ("recovery.open", serde_json::json!({ "id": id })),
            RecoveryAction::Discard(_) | RecoveryAction::DiscardAll
                if self.discard_armed.as_ref() != Some(&action) =>
            {
                self.discard_armed = Some(action);
                return;
            }
            RecoveryAction::Discard(id) => ("recovery.discard", serde_json::json!({ "id": id })),
            RecoveryAction::DiscardAll => ("recovery.discard", serde_json::json!({ "all": true })),
        };
        self.discard_armed = None;
        let outcome = session.execute(command, &params);
        let (ok, text) = match &outcome {
            Ok(result) => (true, result.to_string()),
            Err(error) => (false, error.message().to_string()),
        };
        self.last_command = Some(CommandReport {
            label: command.to_string(),
            ok,
            text: text.chars().take(160).collect(),
        });
        self.sync(session);
    }

    /// The bar under the menu that offers what a crashed 3DCraft left
    /// behind. Returns the button that was clicked.
    fn recovery_bar(&mut self, ui: &mut egui::Ui) -> Option<RecoveryAction> {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        let now = epoch_now();
        let mut hits = Vec::new();
        let mut clicked = None;
        let mut button = |ui: &mut egui::Ui, action: RecoveryAction, label: &str, enabled: bool| {
            let response = ui.add_enabled(enabled, egui::Button::new(label));
            if response.clicked() {
                clicked = Some(action.clone());
            }
            hits.push((action, response.rect));
        };
        let armed = |action: &RecoveryAction| self.discard_armed.as_ref() == Some(action);
        let count = self.recovery.found.len();
        ui.add_space(tokens.inset * 0.3);
        ui.horizontal(|ui| {
            let what = if count == 1 {
                "an unsaved document was".to_string()
            } else {
                format!("{count} unsaved documents were")
            };
            ui.label(
                egui::RichText::new(format!(
                    "3DCraft did not close properly: {what} autosaved and can be recovered."
                ))
                .strong()
                .color(color(tokens.attention)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                button(ui, RecoveryAction::Later, "Later", true);
                let all = RecoveryAction::DiscardAll;
                let label = if armed(&all) {
                    "Discard all for good?"
                } else {
                    "Discard all"
                };
                button(ui, all, label, true);
            });
        });
        for found in self.recovery.found.iter().take(RECOVERY_ROWS) {
            ui.horizontal(|ui| {
                button(
                    ui,
                    RecoveryAction::Open(found.id.clone()),
                    "Recover",
                    found.openable,
                );
                let discard = RecoveryAction::Discard(found.id.clone());
                let label = if armed(&discard) {
                    "Discard for good?"
                } else {
                    "Discard"
                };
                button(ui, discard, label, true);
                ui.label(egui::RichText::new(&found.name).strong());
                let detail = recoverable_text(found, now);
                ui.add(egui::Label::new(egui::RichText::new(detail).color(dim)).truncate());
            });
        }
        if count > RECOVERY_ROWS {
            ui.colored_label(
                dim,
                format!(
                    "and {} more (the command recovery.list shows them all)",
                    count - RECOVERY_ROWS
                ),
            );
        }
        ui.add_space(tokens.inset * 0.3);
        self.recovery_hits = hits;
        clicked
    }

    fn status_bar(&self, ui: &mut egui::Ui, status: Option<&DocumentStatus>) {
        let dim = color(self.tokens.text_dim);
        ui.horizontal(|ui| {
            match status {
                Some(status) => {
                    ui.label(format!("{} · {}", status.length_unit, status.angle_unit));
                    ui.separator();
                    let name = status.path.as_deref().unwrap_or("untitled");
                    let dirty = if status.dirty { " *" } else { "" };
                    ui.label(format!("{name}{dirty}"));
                }
                None => {
                    ui.colored_label(dim, "no document");
                }
            }
            if let Some((text, problem)) = autosave_text(&self.recovery, epoch_now()) {
                ui.separator();
                let tint = if problem {
                    color(self.tokens.failed)
                } else {
                    dim
                };
                let label = egui::Label::new(egui::RichText::new(&text).color(tint)).truncate();
                // The whole reason, when the bar is too narrow for it.
                ui.add(label).on_hover_text(&text);
                if !problem {
                    // "3 min ago" grows old without anything else to draw.
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs(30));
                }
            }
            if let Some(report) = &self.last_command {
                ui.separator();
                let tint = if report.ok {
                    self.tokens.ok
                } else {
                    self.tokens.failed
                };
                ui.colored_label(color(tint), &report.label);
                ui.add(egui::Label::new(egui::RichText::new(&report.text).color(dim)).truncate());
            }
            if let Some(stats) = self.stats {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let time = stats
                        .milliseconds
                        .map_or_else(String::new, |ms| format!(" · {ms:.0} ms"));
                    // The quick first picture is shown while the display
                    // mesh is built: said here, unobtrusively.
                    let refining = if self.shown_scene.as_ref().is_some_and(|s| s.preview) {
                        "refining · "
                    } else {
                        ""
                    };
                    ui.colored_label(
                        dim,
                        format!(
                            "{refining}{} {}×{} · {} triangles{time}",
                            stats.renderer.name().to_uppercase(),
                            stats.width,
                            stats.height,
                            stats.triangles
                        ),
                    );
                });
            }
        });
    }

    fn tree(&mut self, ui: &mut egui::Ui, has_document: bool) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        ui.add_space(tokens.inset * 0.5);
        ui.label(egui::RichText::new("Features").strong());
        ui.separator();
        if self.rows.is_empty() {
            let hint = if has_document {
                "No features yet."
            } else {
                "No document."
            };
            ui.colored_label(dim, hint);
            return;
        }
        let mut clicked = None;
        let mut followed = None;
        let mut hits = Vec::new();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in &self.rows {
                let tint = match row.state {
                    FeatureState::Ok => tokens.ok,
                    FeatureState::Failed => tokens.failed,
                    FeatureState::Suppressed | FeatureState::RolledBack => tokens.inactive,
                };
                let mark = tree_mark(row.state);
                let mut name = egui::RichText::new(&row.name);
                match row.state {
                    FeatureState::Ok => {}
                    FeatureState::Failed => name = name.color(color(tokens.failed)),
                    FeatureState::Suppressed => {
                        name = name.color(color(tokens.inactive)).strikethrough();
                    }
                    FeatureState::RolledBack => name = name.color(color(tokens.inactive)).italics(),
                }
                let hover = match (&row.error, row.state) {
                    (Some(error), _) => error.clone(),
                    (None, FeatureState::Suppressed) => {
                        "Suppressed: not built, and what is made from it fails.".to_string()
                    }
                    (None, FeatureState::RolledBack) => {
                        "Rolled back: after the rollback marker, not built.".to_string()
                    }
                    (None, _) => format!("{} · {}", row.kind, row.status),
                };
                ui.horizontal(|ui| {
                    ui.colored_label(color(tint), mark).on_hover_text(&hover);
                    if ui
                        .selectable_label(row.selected, name)
                        .on_hover_text(&hover)
                        .clicked()
                    {
                        clicked = Some(row.id.clone());
                    }
                    ui.colored_label(dim, format!("{} · {}", row.kind, row.status));
                });
                if let Some((cause, cause_name)) = &row.caused_by {
                    ui.indent(("craft3d-cause", &row.id), |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(dim, "caused by");
                            let link = ui
                                .link(egui::RichText::new(cause_name).color(color(tokens.failed)))
                                .on_hover_text("Select the feature to fix first");
                            hits.push((row.id.clone(), cause.clone(), link.rect));
                            if link.clicked() {
                                followed = Some(cause.clone());
                            }
                        });
                    });
                }
                if let Some(error) = &row.error {
                    ui.indent(("craft3d-error", &row.id), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(error)
                                    .color(color(tokens.failed))
                                    .small(),
                            )
                            .wrap(),
                        );
                    });
                }
            }
        });
        self.cause_hits = hits;
        // A click on a cause selects that feature (it does not toggle: the
        // point is to get there).
        if let Some(id) = followed {
            self.state.selected_feature = Some(id);
            return;
        }
        // Clicking the selected row again clears the selection. Which row
        // is highlighted is view state: nothing in the document changes.
        if let Some(id) = clicked {
            self.state.selected_feature = if self.state.selected_feature.as_ref() == Some(&id) {
                None
            } else {
                Some(id)
            };
        }
    }

    /// The work in progress, over the middle of the viewport: a spinner,
    /// what is being done, and how far along the steps are.
    fn work_plate(&self, ui: &egui::Ui, painter: &egui::Painter, inner: egui::Rect, work: &Work) {
        let tokens = self.tokens;
        let text = work.describe();
        let galley = painter.layout_no_wrap(
            text,
            egui::FontId::proportional(15.0),
            color(tokens.notice_text),
        );
        let text_size = galley.size();
        let spinner = text_size.y * 1.2;
        let gap = tokens.inset * 0.6;
        let bar = tokens.work_bar;
        let width = (spinner + gap + text_size.x).max(tokens.slider_width);
        let height = spinner.max(text_size.y) + gap + bar;
        let plate = egui::Rect::from_center_size(inner.center(), egui::vec2(width, height))
            .expand(tokens.inset * 0.8);
        painter.rect_filled(
            plate,
            egui::CornerRadius::same(tokens.notice_radius),
            color_alpha(tokens.notice_fill, tokens.notice_opacity),
        );
        let top = plate.shrink(tokens.inset * 0.8);
        // The spinner: an arc that turns with the clock, so a frozen
        // window would show as a spinner that stopped.
        let centre = egui::pos2(top.left() + spinner * 0.5, top.top() + spinner * 0.5);
        let radius = spinner * 0.38;
        let time = ui.input(|i| i.time);
        let start = (time * 4.0).rem_euclid(std::f64::consts::TAU) as f32;
        let points: Vec<egui::Pos2> = (0..=24)
            .map(|k| {
                let angle = start + k as f32 / 24.0 * std::f32::consts::PI * 1.5;
                centre + radius * egui::vec2(angle.cos(), angle.sin())
            })
            .collect();
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(tokens.work_spinner_width, color(tokens.work_accent)),
        ));
        painter.galley(
            egui::pos2(top.left() + spinner + gap, centre.y - text_size.y * 0.5),
            galley,
            color(tokens.notice_text),
        );
        let track = egui::Rect::from_min_size(
            egui::pos2(top.left(), top.bottom() - bar),
            egui::vec2(top.width(), bar),
        );
        let round = egui::CornerRadius::same(tokens.radius);
        painter.rect_filled(track, round, color(tokens.work_track));
        if let Some(fraction) = work.fraction() {
            let done = egui::Rect::from_min_size(
                track.min,
                egui::vec2(track.width() * fraction, track.height()),
            );
            painter.rect_filled(done, round, color(tokens.work_accent));
        }
        // The spinner turns: have the next frame come.
        ui.ctx().request_repaint();
    }

    fn viewport(&mut self, ui: &mut egui::Ui, shared: Option<&Arc<Scene>>) {
        let scene = shared.map(Arc::as_ref);
        let tokens = self.tokens;
        let (rect, response) =
            ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let scale = ui.pixels_per_point();
        self.rect = Some(rect);
        self.pixels_per_point = scale;
        // Rounded and clamped to a usable image size.
        let side = |points: f32| {
            let pixels = (f64::from(points) * f64::from(scale)).round();
            if pixels.is_finite() {
                (pixels.clamp(f64::from(MIN_SIDE), f64::from(MAX_SIDE))) as u32
            } else {
                MIN_SIDE
            }
        };
        let size = [side(rect.width()), side(rect.height())];
        // A window being resized changes size every frame: those frames are
        // drawn like a moving camera, and the sharp image follows.
        let resized = size != self.size && self.texture.is_some();
        if size != self.size {
            self.size = size;
            // The model was framed for another shape of viewport.
            self.framed = Bbox3::EMPTY;
            if self.state.camera.auto_fit
                && let Some(scene) = scene
            {
                self.framed = scene.bbox;
                let aspect = self.aspect();
                self.state.camera.fit(&scene.bbox, aspect);
            }
        }

        let turning = self.advance_turn(ui, scene);
        let moving = self.pointer(ui, &response, rect, scene) || resized || turning;
        // A click that was not a drag points at what is under it.
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            let [width, height] = self.size;
            let across = |from: f32, to: f32, at: f32, pixels: u32| {
                f64::from(at - from) / f64::from((to - from).max(f32::MIN_POSITIVE))
                    * f64::from(pixels)
            };
            let x = across(rect.left(), rect.right(), at.x, width);
            let y = across(rect.top(), rect.bottom(), at.y, height);
            // A picture that cannot be made has nothing to point at; the
            // viewport says why already.
            let _ = self.pick(scene, x, y, None, None, true);
            ui.ctx().request_repaint();
        }

        // A quick preview whenever what is shown changes, and the sharp
        // image once the camera rests (see `frames`).
        let largest = ui.input(|i| i.max_texture_side);
        let key = FrameKey {
            revision: scene.map(|s| s.revision),
            tolerance: scene.map(Scene::sampling),
            camera: self.state.camera,
            size,
            largest,
            selected: self.state.selected_feature.clone(),
            highlight: self.face_highlight(scene),
            highlight_edge: self.state.highlight_edge.clone(),
            highlight_vertex: self.state.highlight_vertex.clone(),
            opacity: self.opacity(),
            section: self.section(),
            datums: self.state.show.datums,
            sketches: self.state.show.sketches,
            tokens,
        };
        // The direct path draws the whole picture, lines included, on the
        // window's device, every frame in full; the other paths make a
        // preview here and the sharp image elsewhere.
        self.direct_shown = self.direct_frame(scene, &key);
        if !self.direct_shown {
            self.refresh(ui.ctx(), shared, &key, moving);
        }
        if moving {
            // One more frame after the last movement, for the sharp image.
            ui.ctx().request_repaint();
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, color(tokens.viewport_background));
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        if let Some(id) = self.direct_texture() {
            painter.image(id, rect, uv, egui::Color32::WHITE);
        } else {
            if let Some(texture) = &self.texture {
                painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
            }
            paint_overlay(&painter, rect, self.overlay_size, &self.overlay, &tokens);
        }
        // The names of the datums, on either path: text is egui's.
        if self.state.show.datums {
            let shown = viewport::datum_marks(scene, true);
            let [w, h] = size;
            for (at, name) in viewport::datum_labels(&shown, &self.state.camera, w, h) {
                let place = egui::pos2(
                    rect.left() + (at[0] / f64::from(w.max(1))) as f32 * rect.width(),
                    rect.top() + (at[1] / f64::from(h.max(1))) as f32 * rect.height(),
                );
                if rect.contains(place) {
                    painter.text(
                        place + egui::vec2(tokens.datum_point_radius + 2.0, -2.0),
                        egui::Align2::LEFT_BOTTOM,
                        name,
                        egui::FontId::proportional(12.0),
                        color(tokens.datum),
                    );
                }
            }
        }

        let inset = tokens.inset;
        let inner = rect.shrink(inset);
        let hint = |text: &str| {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(15.0),
                color(tokens.text_dim),
            );
        };
        match (scene, self.problem()) {
            (None, None) => hint("No document. Run file.new or file.open."),
            (Some(scene), _) if scene.bbox.is_empty() => hint("Empty document."),
            _ => {}
        }
        if let Some(problem) = self.problem() {
            painter.text(
                inner.left_bottom(),
                egui::Align2::LEFT_BOTTOM,
                problem,
                egui::FontId::proportional(13.0),
                color(tokens.failed),
            );
        }
        if !self.state.notice.is_empty() {
            let galley = painter.layout_no_wrap(
                self.state.notice.clone(),
                egui::FontId::proportional(17.0),
                color(tokens.notice_text),
            );
            let text_size = galley.size();
            let center = egui::pos2(rect.center().x, inner.top() + text_size.y);
            let plate = egui::Rect::from_center_size(center, text_size).expand(inset * 0.8);
            painter.rect_filled(
                plate,
                egui::CornerRadius::same(tokens.notice_radius),
                color_alpha(tokens.notice_fill, tokens.notice_opacity),
            );
            painter.galley(
                egui::pos2(center.x - text_size.x * 0.5, center.y - text_size.y * 0.5),
                galley,
                color(tokens.notice_text),
            );
        }

        if let Some(work) = &self.work {
            self.work_plate(ui, &painter, inner, work);
        }

        // The view cube and, under it, the controls the view buttons
        // had that it does not replace: fit, the corner view, the
        // projection. Bottom-right, camera only, so not commands. They are
        // widgets of their own over the viewport: a click on them is not
        // a click in the viewport, and picks nothing.
        let mut view = None;
        let mut fit = false;
        let bar = egui::Rect::from_min_max(
            egui::pos2(inner.left(), inner.bottom() - tokens.bar_height),
            inner.right_bottom(),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("iso").clicked() {
                    view = Some("iso");
                }
                fit = ui.small_button("Fit").clicked();
                // How see-through the bodies are: view state, like the
                // camera. Its own widget, so a drag on it is not a drag
                // in the viewport.
                ui.add_space(8.0);
                let mut transparency = self.state.transparency;
                ui.spacing_mut().slider_width = tokens.slider_width;
                let slider = ui
                    .add(
                        egui::Slider::new(&mut transparency, 0.0..=crate::state::MAX_TRANSPARENCY)
                            .show_value(false),
                    )
                    .on_hover_text("See-through: how much of what is behind a surface shows");
                self.transparency_rect = Some(slider.rect);
                if slider.changed() && transparency.is_finite() {
                    self.state.transparency =
                        transparency.clamp(0.0, crate::state::MAX_TRANSPARENCY);
                    ui.ctx().request_repaint();
                }
                ui.label(
                    egui::RichText::new("See-through")
                        .color(color(tokens.text_dim))
                        .small(),
                );
            });
        });
        let side = tokens
            .view_cube_size
            .min(inner.width())
            .min(inner.height() - tokens.bar_height - tokens.bar_gap);
        let cube_shown = side >= 4.0 * tokens.view_cube_dot;
        if cube_shown {
            let place = egui::Rect::from_min_size(
                egui::pos2(inner.right() - side, bar.top() - tokens.bar_gap - side),
                egui::vec2(side, side),
            );
            self.view_cube_rect = Some(place);
            self.view_cube(ui, place, scene);
        } else {
            self.view_cube_rect = None;
            self.cube_drag_from = None;
        }
        // The section, in a row above the bar, beside the view cube.
        let cube_left = if cube_shown {
            inner.right() - side - tokens.bar_gap
        } else {
            inner.right()
        };
        let strip = egui::Rect::from_min_max(
            egui::pos2(inner.left(), bar.top() - tokens.bar_gap - tokens.bar_height),
            egui::pos2(cube_left, bar.top() - tokens.bar_gap),
        );
        if strip.width() > tokens.slider_width && strip.top() > inner.top() {
            self.section_control(ui, strip, scene);
        } else {
            self.section_hits.clear();
        }
        // What is drawn besides the bodies, in the row above.
        let above = strip.translate(egui::vec2(0.0, -(tokens.bar_height + tokens.bar_gap)));
        if above.width() > tokens.slider_width && above.top() > inner.top() {
            self.show_control(ui, above);
        } else {
            self.show_hits.clear();
        }
        if let Some(name) = view {
            self.turn = None;
            // The names come from the table, so this cannot fail.
            let _ = self.state.camera.set_view(name);
            ui.ctx().request_repaint();
        }
        if fit || view.is_some() {
            self.fit_to(scene);
            ui.ctx().request_repaint();
        }
    }

    /// The toggles of what the viewport draws besides the bodies, in
    /// `strip` from the right: Sketches and Datums ([`UiState::show`],
    /// which `ui.set` sets too). Widgets over the viewport, like the
    /// section's: a click on them picks nothing.
    fn show_control(&mut self, ui: &mut egui::Ui, strip: egui::Rect) {
        let mut show = self.state.show;
        let mut hits: Vec<(&'static str, egui::Rect)> = Vec::new();
        ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let datums = ui
                    .add(egui::Button::new("Datums").small().selected(show.datums))
                    .on_hover_text("Show or hide the datum planes, axes and points");
                hits.push(("datums", datums.rect));
                if datums.clicked() {
                    show.datums = !show.datums;
                }
                let sketches = ui
                    .add(
                        egui::Button::new("Sketches")
                            .small()
                            .selected(show.sketches),
                    )
                    .on_hover_text("Show or hide the sketches");
                hits.push(("sketches", sketches.rect));
                if sketches.clicked() {
                    show.sketches = !show.sketches;
                }
            });
        });
        self.show_hits = hits;
        if show != self.state.show {
            self.state.show = show;
            ui.ctx().request_repaint();
        }
    }

    /// The section's controls in `strip`, from the right: the toggle and,
    /// while the section is on, the axis the plane is across (X, Y, Z),
    /// a slider that moves the plane across the model's extent, and Flip,
    /// which keeps the other half. View state, like the camera: they set
    /// [`UiState::section`], which `ui.set` sets too. They are widgets of
    /// their own over the viewport: a click on them picks nothing.
    fn section_control(&mut self, ui: &mut egui::Ui, strip: egui::Rect, scene: Option<&Scene>) {
        let tokens = self.tokens;
        let mut section = self.state.section.clone();
        let range = self.section_cache.as_ref().and_then(|cache| cache.range);
        // The middle of the model along a base plane's normal: where a
        // plane that is chosen here starts.
        let middle = |axis: usize| {
            scene
                .and_then(|scene| scene.bbox.corners())
                .map_or(0.0, |(lo, hi)| {
                    let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
                    match (lo.get(axis), hi.get(axis)) {
                        (Some(lo), Some(hi)) => (lo + hi) * 0.5,
                        _ => 0.0,
                    }
                })
        };
        let mut hits: Vec<(&'static str, egui::Rect)> = Vec::new();
        ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let toggle = ui
                    .add(egui::Button::new("Section").small().selected(section.on))
                    .on_hover_text("Cut the picture open with a plane, to look inside");
                hits.push(("toggle", toggle.rect));
                if toggle.clicked() {
                    section.on = !section.on;
                    if section.on && section.plane.is_none() && section.normal.is_none() {
                        section.plane = Some("xy".to_string());
                        section.offset = middle(2);
                    }
                }
                if !section.on {
                    return;
                }
                // Right to left, so that they read X, Y, Z.
                for (label, name, plane, axis) in [
                    ("Z", "z", "xy", 2),
                    ("Y", "y", "zx", 1),
                    ("X", "x", "yz", 0),
                ] {
                    let chosen = section.plane.as_deref() == Some(plane);
                    let button = ui
                        .add(egui::Button::new(label).small().selected(chosen))
                        .on_hover_text(format!("Cut across {label}"));
                    hits.push((name, button.rect));
                    if button.clicked() && !chosen {
                        section.plane = Some(plane.to_string());
                        section.feature = None;
                        section.point = None;
                        section.normal = None;
                        section.offset = middle(axis);
                    }
                }
                if let Some([lo, hi]) = range.filter(|[lo, hi]| lo < hi) {
                    let mut offset = section.offset.clamp(lo, hi);
                    ui.spacing_mut().slider_width = tokens.slider_width;
                    let slider = ui
                        .add(egui::Slider::new(&mut offset, lo..=hi).show_value(false))
                        .on_hover_text("Where the plane cuts: across the model");
                    hits.push(("offset", slider.rect));
                    if slider.changed() && offset.is_finite() {
                        section.offset = offset;
                    }
                }
                let flip = ui.small_button("Flip").on_hover_text("Keep the other half");
                hits.push(("flip", flip.rect));
                if flip.clicked() {
                    section.flip = !section.flip;
                }
            });
        });
        self.section_hits = hits;
        if section != self.state.section
            && let Ok(section) = section.validated()
        {
            self.state.section = section;
            self.section_lost = None;
            ui.ctx().request_repaint();
        }
    }

    /// The view cube in `place`: paints it and handles its mouse. A click
    /// on a facet starts a turn of the camera to that side (see
    /// [`Self::advance_turn`]), a click on the ring around the cube
    /// switches between perspective and orthographic, and a drag orbits
    /// the camera.
    fn view_cube(&mut self, ui: &mut egui::Ui, place: egui::Rect, scene: Option<&Scene>) {
        let tokens = self.tokens;
        let response = ui.interact(
            place,
            ui.id().with("view-cube"),
            egui::Sense::click_and_drag(),
        );
        // The radius of the cube's picture, in points: the ring and the
        // ends of the axis sticks are outside it.
        let arm = (place.width() * 0.5 / 1.3).max(1.0);
        let center = place.center();
        let unit = |at: egui::Pos2| {
            [
                f64::from(at.x - center.x) / f64::from(arm),
                f64::from(at.y - center.y) / f64::from(arm),
            ]
        };
        if response.drag_started() {
            self.cube_drag_from = Some(self.state.camera);
            self.turn = None;
        }
        if response.dragged() {
            // As a drag in the viewport: from where it began, by how far
            // the pointer is from there now.
            if let (Some(from), Some(total)) = (self.cube_drag_from, response.total_drag_delta()) {
                let mut camera = from;
                camera.orbit(f64::from(total.x), f64::from(total.y));
                self.state.camera = camera;
            }
            ui.ctx().request_repaint();
        } else {
            self.cube_drag_from = None;
        }
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            match view_cube::facet_at(&self.state.camera, unit(at)) {
                Some(facet) => {
                    let side = view_cube::clicked(&self.state.camera, &facet);
                    let mut to = view_cube::turned(&self.state.camera, side);
                    // Framed there, as the view buttons framed.
                    if let Some(scene) = scene.filter(|scene| !scene.bbox.is_empty()) {
                        to.fit(&scene.bbox, self.aspect());
                        to.auto_fit = true;
                    }
                    self.turn = Some(Turn {
                        from: self.state.camera,
                        to,
                        started: ui.input(|i| i.time),
                        last: self.state.camera,
                    });
                }
                // Beside the cube, on the ring: the other projection.
                None => {
                    self.turn = None;
                    self.state.camera.projection = match self.state.camera.projection {
                        ProjectionKind::Perspective => ProjectionKind::Orthographic,
                        ProjectionKind::Orthographic => ProjectionKind::Perspective,
                    };
                }
            }
            ui.ctx().request_repaint();
        }
        let hover = response.hover_pos().map(unit);
        let hovered = hover.and_then(|at| view_cube::facet_at(&self.state.camera, at));
        let on_ring = hover.is_some() && hovered.is_none();
        let painter = ui.painter_at(place.expand(1.0));
        let at = |p: [f64; 2]| center + egui::vec2(p[0] as f32, p[1] as f32) * arm;
        let mix = |a: [u8; 3], b: [u8; 3], t: f64| -> [u8; 3] {
            let t = t.clamp(0.0, 1.0);
            let mut out = [0; 3];
            for ((slot, x), y) in out.iter_mut().zip(a).zip(b) {
                // Between two bytes.
                *slot = (f64::from(x) + (f64::from(y) - f64::from(x)) * t).round() as u8;
            }
            out
        };
        // The ring: the projection's switch, with its name along the
        // bottom. Brighter under the pointer.
        let ring = place.width() * 0.5;
        painter.circle(
            center,
            ring - 1.0,
            color_alpha(tokens.panel_fill, tokens.view_cube_opacity),
            egui::Stroke::new(
                if on_ring { 2.0 } else { 1.0 },
                color(if on_ring {
                    tokens.text
                } else {
                    tokens.text_dim
                }),
            ),
        );
        let projection = match self.state.camera.projection {
            ProjectionKind::Perspective => "perspective",
            ProjectionKind::Orthographic => "orthographic",
        };
        painter.text(
            egui::pos2(center.x, place.bottom() - 3.0),
            egui::Align2::CENTER_BOTTOM,
            projection,
            egui::FontId::proportional(9.5),
            color(if on_ring {
                tokens.text
            } else {
                tokens.text_dim
            }),
        );
        let stick = |axis: usize, front: bool| {
            if let Some((from, to, toward)) = view_cube::axis_stick(&self.state.camera, axis)
                && toward == front
            {
                let ink = tokens.axes.get(axis).copied().unwrap_or(tokens.text_dim);
                painter.line_segment(
                    [at(from), at(to)],
                    egui::Stroke::new(tokens.axis_width * 1.6, color(ink)),
                );
                painter.circle_filled(at(to), tokens.view_cube_dot * 0.45, color(ink));
            }
        };
        // The sticks that point away first, then the gem, then the ones
        // that point at the viewer.
        for axis in 0..3 {
            stick(axis, false);
        }
        for facet in view_cube::facets(&self.state.camera) {
            let points: Vec<egui::Pos2> = facet.outline.iter().map(|&p| at(p)).collect();
            // A face carries the colour of the axis it is across; the cut
            // facets are the gem's own, darker, and all are lit by how
            // much they face the viewer.
            let base = match facet.axis() {
                Some(axis) => mix(
                    tokens.body,
                    tokens.axes.get(axis).copied().unwrap_or(tokens.body),
                    0.38,
                ),
                None => mix(tokens.panel_fill, tokens.body, 0.45),
            };
            let lit = mix(tokens.notice_fill, base, 0.45 + 0.55 * facet.toward);
            let over = hovered.as_ref().is_some_and(|h| h.side == facet.side);
            let fill = if over {
                mix(lit, tokens.face_highlight, 0.75)
            } else {
                lit
            };
            painter.add(egui::Shape::convex_polygon(
                points,
                color(fill),
                egui::Stroke::new(1.0, color(tokens.edge)),
            ));
            if let Some(label) = facet.label().filter(|_| facet.toward > 0.55) {
                painter.text(
                    at(facet.center()),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(arm * 0.21),
                    color_alpha(
                        tokens.notice_fill,
                        (255.0 * facet.toward.clamp(0.0, 1.0)) as u8,
                    ),
                );
            }
        }
        for axis in 0..3 {
            stick(axis, true);
        }
    }

    /// Moves the camera along the turn a click on the view cube started:
    /// a quarter of a second from where it was to the view asked for,
    /// framed. Anything else that moves the camera meanwhile (a drag,
    /// `ui.set`) ends the turn where it is. Returns true while it turns.
    fn advance_turn(&mut self, ui: &egui::Ui, scene: Option<&Scene>) -> bool {
        let Some(turn) = self.turn else {
            return false;
        };
        if self.state.camera != turn.last {
            self.turn = None;
            return false;
        }
        let elapsed = ui.input(|i| i.time) - turn.started;
        let camera = view_cube::between(&turn.from, &turn.to, elapsed / TURN_SECONDS);
        self.state.camera = camera;
        ui.ctx().request_repaint();
        if camera == turn.to {
            self.turn = None;
            if let Some(scene) = scene.filter(|_| turn.to.auto_fit) {
                self.framed = scene.bbox;
            }
            return false;
        }
        self.turn = Some(Turn {
            last: camera,
            ..turn
        });
        true
    }

    /// Where the view cube is in the window, in points, once the
    /// viewport was drawn with room for it.
    pub fn view_cube_rect(&self) -> Option<egui::Rect> {
        self.view_cube_rect
    }

    fn fit_to(&mut self, scene: Option<&Scene>) {
        if let Some(scene) = scene
            && !scene.bbox.is_empty()
        {
            let aspect = self.aspect();
            self.state.camera.fit(&scene.bbox, aspect);
            self.state.camera.auto_fit = true;
            self.framed = scene.bbox;
        }
    }

    /// Mouse and keys over the viewport. Returns true while the camera is
    /// being moved.
    fn pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: egui::Rect,
        scene: Option<&Scene>,
    ) -> bool {
        let mut moving = false;
        if response.drag_started() {
            self.drag_from = Some(self.state.camera);
        }
        if response.dragged() {
            // From where the drag began, by how far the pointer is from
            // there now: the result does not depend on how the movement was
            // cut into frames.
            if let (Some(from), Some(total)) = (self.drag_from, response.total_drag_delta()) {
                let (primary, secondary, middle, shift) = ui.input(|i| {
                    (
                        i.pointer.button_down(egui::PointerButton::Primary),
                        i.pointer.button_down(egui::PointerButton::Secondary),
                        i.pointer.button_down(egui::PointerButton::Middle),
                        i.modifiers.shift,
                    )
                });
                let mut camera = from;
                let (dx, dy) = (f64::from(total.x), f64::from(total.y));
                if secondary || middle || (primary && shift) {
                    camera.pan(dx, dy, f64::from(rect.height()));
                } else if primary {
                    camera.orbit(dx, dy);
                }
                self.state.camera = camera;
            }
            moving = true;
        } else {
            self.drag_from = None;
        }
        if response.contains_pointer() {
            let (scroll, fit) = ui.input(|i| {
                (
                    f64::from(i.smooth_scroll_delta().y),
                    i.key_pressed(egui::Key::F),
                )
            });
            if scroll != 0.0 {
                self.state.camera.zoom(scroll);
                moving = true;
            }
            if fit {
                self.fit_to(scene);
            }
        }
        moving
    }

    /// The direct path's texture, when the picture on screen is its.
    fn direct_texture(&self) -> Option<egui::TextureId> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.direct
                .as_ref()
                .filter(|_| self.direct_shown)
                .and_then(Direct::texture_id)
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    /// Draws `key` on the window's device, when the shell has it. True
    /// when the picture on screen is now the direct path's; false when
    /// there is no direct path, or this frame has to be made another way.
    #[cfg(not(target_arch = "wasm32"))]
    fn direct_frame(&mut self, scene: Option<&Scene>, key: &FrameKey) -> bool {
        let cut = self.section_cut().filter(|_| key.section.is_some());
        let Some(direct) = &mut self.direct else {
            return false;
        };
        direct.set_highlight(key.highlight);
        direct.set_opacity(key.opacity);
        direct.set_marks(marks_of(
            scene,
            key.highlight_edge.as_ref(),
            key.highlight_vertex.as_ref(),
            cut,
            key.datums,
            key.sketches,
        ));
        let outcome = direct.frame(
            scene,
            &key.camera,
            key.size,
            key.selected.as_deref(),
            &key.tokens,
            self.pixels_per_point,
        );
        match outcome {
            Ok(None) => true,
            Ok(Some(frame)) => {
                self.renders = self.renders.saturating_add(1);
                self.stats = Some(RenderStats {
                    width: frame.width,
                    height: frame.height,
                    ssaa: 1,
                    triangles: frame.triangles,
                    milliseconds: Some(frame.milliseconds),
                    sharp: true,
                    renderer: Backend::Gpu,
                    msaa: frame.msaa,
                });
                self.shown_report = Some(direct.report());
                self.render_problem = None;
                true
            }
            Err(DirectFailure::Frame(why)) => {
                // The CPU would refuse it too: nothing to fall back to.
                self.render_problem = Some(format!("the viewport cannot be rendered: {why}"));
                true
            }
            // This frame the other way; the next one here again.
            Err(DirectFailure::ThisFrame(_)) => false,
            Err(DirectFailure::ForGood(why)) => {
                self.direct = None;
                self.direct_fallback = Some(why);
                false
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn direct_frame(&mut self, _scene: Option<&Scene>, _key: &FrameKey) -> bool {
        false
    }

    /// Brings the image on screen up to date with `key`: makes a preview
    /// here when nothing of the key is on screen, starts the sharp image,
    /// and shows the sharp images that arrived and are still current.
    fn refresh(
        &mut self,
        ctx: &egui::Context,
        scene: Option<&Arc<Scene>>,
        key: &FrameKey,
        moving: bool,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        if self.worker.as_ref().is_some_and(|worker| !worker.alive()) {
            // The thread is gone (it cannot be, but if): go on without it
            // rather than wait for images that will not come.
            self.worker = None;
            self.scheduler.set_mode(Mode::Inline);
        }
        if self.texture.is_none() && self.scheduler.shown().is_some() {
            // The texture was lost: what the scheduler believes to be on
            // screen is not. (Only then: forgetting every frame while no
            // image can be made would ask for the sharp one every frame.)
            self.scheduler.forget();
        }
        let cut = self.section_cut().filter(|_| key.section.is_some());
        let job = |quality: Quality| RenderJob {
            scene: scene.cloned(),
            camera: key.camera,
            quality,
            tokens: key.tokens,
            selected: key.selected.clone(),
            highlight: key.highlight,
            marks: marks_of(
                scene.map(Arc::as_ref),
                key.highlight_edge.as_ref(),
                key.highlight_vertex.as_ref(),
                cut.clone(),
                key.datums,
                key.sketches,
            ),
            opacity: key.opacity,
            #[cfg(not(target_arch = "wasm32"))]
            wake: Some(ctx.clone()),
        };
        let plan = self.scheduler.plan(key, moving);
        let backend = self.renderer.backend();
        if plan.preview {
            let job = job(image_quality_for(backend, key.size, true, key.largest));
            match job.render(&mut self.renderer) {
                Ok(rendered) => {
                    self.show(ctx, rendered, false);
                    self.scheduler.preview_shown(key);
                }
                Err(e) => {
                    self.render_problem = Some(format!("the viewport cannot be rendered: {e}"));
                }
            }
        }
        if let Some(ticket) = plan.sharp {
            let job = job(image_quality_for(backend, key.size, false, key.largest));
            if let Some(job) = self.hand_over(ticket, job) {
                // No thread to do it: here and now.
                let result = job.render(&mut self.renderer);
                self.arrived.push((ticket, result));
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(worker) = &self.worker {
            while let Some(done) = worker.poll() {
                self.arrived.push((done.ticket, done.result));
            }
        }
        for (ticket, result) in std::mem::take(&mut self.arrived) {
            match result {
                // Stale images are dropped here: `deliver` accepts only
                // the image of what this frame shows.
                Ok(rendered) => {
                    if self.scheduler.deliver(ticket) {
                        self.show(ctx, rendered, true);
                    }
                }
                Err(e) => {
                    if self.scheduler.fail(ticket) {
                        self.render_problem = Some(format!("the viewport cannot be rendered: {e}"));
                    }
                }
            }
        }
    }

    /// Gives the job to the render thread. Hands it back when there is no
    /// thread to take it.
    #[cfg(not(target_arch = "wasm32"))]
    fn hand_over(&mut self, ticket: Ticket, job: RenderJob) -> Option<RenderJob> {
        match &self.worker {
            Some(worker) => worker.submit(ticket, job).err(),
            None => Some(job),
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn hand_over(&mut self, _ticket: Ticket, job: RenderJob) -> Option<RenderJob> {
        Some(job)
    }

    /// Puts an image on screen.
    fn show(&mut self, ctx: &egui::Context, rendered: Rendered, sharp: bool) {
        let Rendered {
            composed,
            quality,
            milliseconds,
            report,
        } = rendered;
        self.renders = self.renders.saturating_add(1);
        self.stats = Some(RenderStats {
            width: composed.width,
            height: composed.height,
            ssaa: quality.ssaa,
            triangles: composed.triangles,
            milliseconds,
            sharp,
            renderer: composed.backend,
            msaa: 1,
        });
        self.shown_report = Some(report);
        let image = egui::ColorImage::from_rgb(
            [composed.width as usize, composed.height as usize],
            &composed.rgb,
        );
        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture =
                    Some(ctx.load_texture("craft3d-viewport", image, egui::TextureOptions::LINEAR));
            }
        }
        self.overlay = composed.overlay;
        self.overlay_size = [composed.width, composed.height];
        self.render_problem = None;
    }
}

/// How to render the image for a viewport of `size` pixels on the CPU:
/// full size at rest, supersampled when that fits [`REST_SAMPLES`];
/// [`MOVING_DIVISOR`] times smaller and unfiltered while the camera moves,
/// within [`MOVING_SAMPLES`]. Never larger than the backend's largest
/// texture (`largest` pixels per side). An image that has to be smaller
/// than the viewport keeps its shape and is stretched over it.
pub fn image_quality(size: [u32; 2], moving: bool, largest: usize) -> Quality {
    image_quality_for(Backend::Cpu, size, moving, largest)
}

/// [`image_quality`] for either renderer: the GPU's budgets count pixels
/// ([`GPU_REST_PIXELS`], [`GPU_MOVING_PIXELS`]), and its image at rest is
/// supersampled whenever the CPU renderer's limits and a target of 8192
/// pixels per side allow it.
pub fn image_quality_for(
    backend: Backend,
    size: [u32; 2],
    moving: bool,
    largest: usize,
) -> Quality {
    let largest = u32::try_from(largest)
        .unwrap_or(MAX_SIDE)
        .clamp(MIN_SIDE, MAX_SIDE);
    let (divisor, budget) = match (backend, moving) {
        (Backend::Cpu, true) => (MOVING_DIVISOR, MOVING_SAMPLES),
        (Backend::Cpu, false) => (1, REST_SAMPLES),
        (Backend::Gpu, true) => (MOVING_DIVISOR, GPU_MOVING_PIXELS),
        (Backend::Gpu, false) => (1, GPU_REST_PIXELS),
    };
    let (mut width, mut height) = (
        (size[0] / divisor).max(MIN_SIDE),
        (size[1] / divisor).max(MIN_SIDE),
    );
    // The same factor for both sides, the smaller of what the texture limit
    // and the sample budget allow.
    let longest = f64::from(width.max(height));
    let pixels = f64::from(width) * f64::from(height);
    let factor = (f64::from(largest) / longest).min((budget as f64 / pixels).sqrt());
    if factor < 1.0 {
        // Rounded down: both products stay within their limit.
        let shrink = |side: u32| (f64::from(side) * factor).floor() as u32;
        (width, height) = (shrink(width), shrink(height));
    }
    let (width, height) = (
        width.clamp(MIN_SIDE, largest),
        height.clamp(MIN_SIDE, largest),
    );
    let supersampled =
        u64::from(width) * u64::from(height) * u64::from(REST_SSAA) * u64::from(REST_SSAA);
    let fits = match backend {
        Backend::Cpu => supersampled <= budget,
        Backend::Gpu => {
            supersampled <= craft3d_render::MAX_SAMPLES
                && u64::from(width.max(height)) * u64::from(REST_SSAA) <= GPU_TARGET_SIDE
        }
    };
    Quality {
        width,
        height,
        ssaa: if !moving && fits { REST_SSAA } else { 1 },
    }
}

/// Every desktop adapter can render into a target this many pixels per
/// side; the GPU renderer checks the device's own limit and the CPU
/// renders the frame when it is smaller.
const GPU_TARGET_SIDE: u64 = 8192;

/// Paints the overlay, which is in image pixels, into `rect`, which is the
/// same image in points.
/// The marks of a highlighted edge and a highlighted vertex, the section
/// the picture is cut open with, and the datums when they are shown.
fn marks_of(
    scene: Option<&Scene>,
    edge: Option<&EdgeRef>,
    vertex: Option<&VertexRef>,
    cut: Option<viewport::SectionCut>,
    datums: bool,
    sketches: bool,
) -> viewport::Marks {
    let edges = edge.map_or_else(viewport::Marks::default, |edge| {
        viewport::edge_marks(scene, edge)
    });
    let vertices = vertex.map_or_else(viewport::Marks::default, |vertex| {
        viewport::vertex_marks(scene, vertex)
    });
    viewport::Marks {
        cut,
        hide_sketches: !sketches,
        ..edges
            .with(vertices)
            .with(viewport::datum_marks(scene, datums))
    }
}

/// A vertex as a highlight refers to it: by its own name when it has one,
/// otherwise by the faces that meet in it. `None` for a vertex with
/// neither.
fn named_vertex(feature: &str, name: Option<&str>, faces: &[String]) -> Option<VertexRef> {
    if name.is_none() && faces.is_empty() {
        return None;
    }
    Some(VertexRef {
        feature: Some(feature.to_string()),
        vertex: name.map(str::to_string),
        corner: if name.is_some() {
            Vec::new()
        } else {
            faces
                .iter()
                .take(crate::state::MAX_CORNER_FACES)
                .cloned()
                .collect()
        },
    })
}

/// The edge a pick found, as a highlight refers to it: by its own name
/// when it has one, otherwise by the faces it lies between (a seam: its
/// one face twice). `None` for an edge with neither.
fn edge_ref(near: &SceneNear) -> Option<EdgeRef> {
    named_edge(&near.feature, near.name.as_deref(), &near.faces)
}

/// [`edge_ref`] for the edge of a remembered click.
fn picked_edge_ref(edge: &PickedEntity) -> Option<EdgeRef> {
    named_edge(&edge.feature, edge.name.as_deref(), &edge.faces)
}

fn named_edge(feature: &str, name: Option<&str>, faces: &[String]) -> Option<EdgeRef> {
    let between = match (name, faces) {
        (Some(_), _) => Vec::new(),
        (None, [a, b]) => vec![a.clone(), b.clone()],
        (None, [a]) => vec![a.clone(), a.clone()],
        (None, _) => return None,
    };
    Some(EdgeRef {
        feature: Some(feature.to_string()),
        edge: name.map(str::to_string),
        between,
    })
}

fn paint_overlay(
    painter: &egui::Painter,
    rect: egui::Rect,
    size: [u32; 2],
    overlay: &Overlay,
    tokens: &Tokens,
) {
    let [w, h] = size;
    let (sx, sy) = (
        f64::from(rect.width()) / f64::from(w.max(1)),
        f64::from(rect.height()) / f64::from(h.max(1)),
    );
    let place = |p: [f64; 2]| {
        egui::pos2(
            rect.left() + (p[0] * sx) as f32,
            rect.top() + (p[1] * sy) as f32,
        )
    };
    for line in &overlay.lines {
        let tint = color_alpha(
            line.kind.color(tokens),
            viewport::opacity(line.hidden, tokens),
        );
        painter.line_segment(
            [place(line.from), place(line.to)],
            egui::Stroke::new(line.kind.width(tokens), tint),
        );
    }
    for point in &overlay.points {
        let tint = color_alpha(
            point.kind.color(tokens),
            viewport::opacity(point.hidden, tokens),
        );
        painter.circle_filled(place(point.at), point.kind.dot_radius(tokens), tint);
    }
}
