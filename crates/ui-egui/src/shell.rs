//! The shell: viewport, feature tree and status bar over one engine
//! session. Thin: it reads the scene and draws it, and owns nothing but
//! view state. It never edits a document; that takes a command
//! (`Session::execute`), from whoever drives the session.

use crate::backend::{Backend, BodyRenderer, RendererChoice, RendererReport};
use crate::direct::{Direct, DirectFailure};
use crate::frames::{Mode, Scheduler, Ticket};
use crate::state::VertexRef;
use crate::state::{
    CameraState, EdgeRef, FaceRef, Picked, PickedEntity, PickedKind, SectionState, TabView,
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
use std::collections::BTreeMap;
use std::sync::Arc;

mod controls;
pub mod help;
mod icons;
mod layout;
pub mod number_field;
mod params;
mod player;
pub mod recent;
mod report;
mod selection;
pub mod start;
mod tabs;
mod timeline;
mod views;

pub use controls::ControlsLayout;
pub use help::{External, Shortcut, ShortcutGroup, ShortcutInput, help_shortcut, shortcuts};
pub use layout::{ControlWidths, controls_layout, triad_side};
pub use params::{
    DEBOUNCE, Faults, NOTE_SECONDS, Pacer, Param, ParamAsk, ParamKind, ParamList, Settled,
    ShellCommand, Verdict, reported_faults, settle, verdict,
};
pub use recent::{MAX_RECENT, RecentEntry, RecentFiles};
pub use start::{AgentSetup, EXAMPLES, Example, run_example};
pub use tabs::{
    CLOSE_MARK, CloseAnswer, DocAction, TabPart, UNSAVED_MARK, close_shortcut, doc_tabs,
    next_shortcut, previous_shortcut, tab_name,
};
pub use views::FLY_SECONDS;
pub(crate) use views::FlyTo;

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
/// needed (`file.new`, `file.open`, `file.save`, `file.import`,
/// `file.export`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    New,
    Open,
    /// To the document's own file; like `SaveAs` when it has none.
    Save,
    SaveAs,
    /// A STEP file read into the active document (`file.import`).
    Import,
    /// The document's bodies, or what is selected, to a file of the
    /// format its extension names (`file.export`).
    Export,
    /// The document published as a Jig, a `.3dcraftjig` file
    /// (`jig.export`): for a document with public parameters.
    ExportJig,
    /// A new untitled model of a Jig in use, to edit (`jig.edit`).
    OpenForEditing,
}

impl FileAction {
    /// In menu order.
    pub const ALL: [FileAction; 8] = [
        FileAction::New,
        FileAction::Open,
        FileAction::OpenForEditing,
        FileAction::Save,
        FileAction::SaveAs,
        FileAction::Import,
        FileAction::Export,
        FileAction::ExportJig,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FileAction::New => "New",
            FileAction::Open => "Open…",
            FileAction::Save => "Save",
            FileAction::SaveAs => "Save As…",
            FileAction::Import => "Import…",
            FileAction::Export => "Export…",
            FileAction::ExportJig => "Export as Jig…",
            FileAction::OpenForEditing => "Open for Editing",
        }
    }

    /// Ctrl (⌘ on a Mac) with N, O, S, Shift+S, I and E; the Jig's
    /// actions have none.
    pub fn shortcut(self) -> Option<egui::KeyboardShortcut> {
        let command = egui::Modifiers::COMMAND;
        let (modifiers, key) = match self {
            FileAction::New => (command, egui::Key::N),
            FileAction::Open => (command, egui::Key::O),
            FileAction::Save => (command, egui::Key::S),
            FileAction::SaveAs => (command | egui::Modifiers::SHIFT, egui::Key::S),
            FileAction::Import => (command, egui::Key::I),
            FileAction::Export => (command, egui::Key::E),
            FileAction::ExportJig | FileAction::OpenForEditing => return None,
        };
        Some(egui::KeyboardShortcut::new(modifiers, key))
    }

    /// Saving and exporting need a document. Importing does not: without
    /// one the host makes a new document to import into.
    pub fn needs_document(self) -> bool {
        matches!(
            self,
            FileAction::Save
                | FileAction::SaveAs
                | FileAction::Export
                | FileAction::ExportJig
                | FileAction::OpenForEditing
        )
    }

    /// The menu draws a line above it.
    fn starts_group(self) -> bool {
        self == FileAction::Import
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
    /// Hidden in the view (`feature.visible`).
    pub hidden: bool,
    /// The first face, edge or vertex selected was made by this feature
    /// (by its persistent name): the row is marked as selected too.
    pub made_selection: bool,
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

/// F pressed without Ctrl, Alt, Shift or Cmd, by the key's own event:
/// Ctrl+F and the like are other shortcuts, not Fit.
fn plain_f(input: &egui::InputState) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: help::FIT_KEY,
                pressed: true,
                modifiers,
                ..
            } if modifiers.is_none()
        )
    }) && input.modifiers.is_none()
}

/// The side panel's inset, in points: its content keeps it from the
/// panel's left edge and from the split on its right (where the lists'
/// scroll bars lie, over that room), the same on both sides.
pub(crate) fn side_inset(tokens: &Tokens) -> i8 {
    tokens.inset.round().clamp(0.0, 64.0) as i8
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
    /// How long it takes: [`TURN_SECONDS`] for the view cube,
    /// [`FLY_SECONDS`] for a flight to a feature or the selection.
    seconds: f64,
}

/// Where a zoom at the pointer was made: the pointer (in points), the
/// camera it left and the point of the model (or of the target plane) it
/// kept under the pointer.
#[derive(Clone, Copy, Debug)]
struct ZoomAnchor {
    pointer: egui::Pos2,
    camera: CameraState,
    point: craft3d_math::Vec3,
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
    /// What is drawn for the selection ([`Shell::selection_drawn`]).
    selection: u64,
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

fn epoch_now() -> Option<u64> {
    #[cfg(not(target_arch = "wasm32"))]
    use std::time::{SystemTime, UNIX_EPOCH};
    #[cfg(target_arch = "wasm32")]
    use web_time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs())
}

pub struct Shell {
    pub state: UiState,
    pub tokens: Tokens,
    pub last_command: Option<CommandReport>,
    /// What "Copy details" reports and whether its window is open
    /// (`shell/report.rs`).
    report: report::ReportState,
    /// Where "Copy details" was drawn in the last frame.
    copy_hit: Option<egui::Rect>,
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
    /// How crowded its edges are, for the pixels per point and the theme
    /// it was measured with: measured once per overlay, not per frame.
    overlay_density: Option<((u64, Tokens), viewport::EdgeDensity)>,
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
    /// Where the see-through field was drawn.
    transparency_rect: Option<egui::Rect>,
    /// A drag on a number field under way, and the field being typed
    /// into.
    fields: number_field::FieldMemory,
    typing: Option<number_field::Typing>,
    /// The active document's length unit, for the Offset field.
    length_unit: &'static str,
    /// Where the section's controls were drawn: `toggle` and, while the
    /// section is on, `x`, `y`, `z`, `offset`, `front` and `back`.
    section_hits: Vec<(&'static str, egui::Rect)>,
    /// Where the Sketches and Datums toggles were drawn.
    show_hits: Vec<(&'static str, egui::Rect)>,
    /// Where Fit, iso and the projection's two halves were drawn.
    camera_hits: Vec<(&'static str, egui::Rect)>,
    /// Where the controls over the viewport were laid out in the last
    /// frame.
    controls_layout: Option<ControlsLayout>,
    /// Where the items of the status bar were drawn.
    status_hits: Vec<(StatusSlot, egui::Rect)>,
    /// The selection as the window draws and changes it.
    select: selection::Selecting,
    /// The history timeline and the rows' eyes (`shell/timeline.rs`).
    timeline: timeline::Timeline,
    /// The history player under the timeline (`shell/player.rs`).
    player: player::Player,
    /// The player's orbit turned the camera this frame.
    orbiting: bool,
    /// Where the feature panel was drawn: the player's keys work over it.
    tree_panel: Option<egui::Rect>,
    /// The point a zoom at the pointer keeps in place, while the pointer
    /// and the camera stay as that zoom left them: one look at the model
    /// per turn of the wheel, not one per frame of it.
    zoom_anchor: Option<ZoomAnchor>,
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
    /// The application was asked to stop without saving (`ui.quit`, or
    /// `ui.quit {save: "discard"}`): the autosaves go too.
    quit_discards: bool,
    /// Closing the window, one question per document with unsaved
    /// changes ([`Self::request_window_close`]).
    quitting: Option<tabs::Quitting>,
    rows: Vec<TreeRow>,
    /// The document's assertions as checked at the last [`Self::sync`],
    /// listed under the features.
    assertion_rows: Vec<craft3d_engine::AssertionState>,
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
    /// The view of every open document but the one shown, by the
    /// document's id, with the box its camera was last fitted to
    /// (`views.rs`).
    views: BTreeMap<u64, (TabView, Bbox3)>,
    /// The document whose view `state` holds.
    viewing: Option<u64>,
    /// A flight asked for, made in the viewport's next frame.
    fly: Option<views::FlyTo>,
    /// The feature that made the first face, edge or vertex selected.
    made_selection: Option<String>,
    /// The row of the feature list to bring into view in the next frame.
    tree_scroll: Option<String>,
    /// The tab was switched and its selection not read yet.
    tab_switched: bool,
    /// Where the last frame drew the name of each row of the feature
    /// list, and the part of the window the list shows its rows in.
    tree_row_hits: Vec<(String, egui::Rect)>,
    tree_view: Option<egui::Rect>,
    /// The Parameters tab: its list, the field being changed, the commit
    /// waiting for the host (`shell/params.rs`).
    params: params::ParamForm,
    /// The Help menu, the recent files and the start screen.
    help: help::HelpMemory,
    recent: recent::RecentMemory,
    start: start::StartMemory,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

/// What an item of the status bar is: the order in which they give way
/// when the bar is too narrow is [`STATUS_YIELD`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusSlot {
    /// The document's units, or "no document".
    Units,
    /// The document's path.
    Path,
    /// When it was autosaved, or why not.
    Autosave,
    /// What is selected: "3 bodies, 2 faces selected".
    Selection,
    /// The last command's name, and what it answered.
    Command,
    Answer,
    /// The renderer's numbers, on the right.
    Render,
}

/// The order in which the items give way: each is first elided to its
/// least width in this order, then left out in this order. The units go
/// last, the answer to the last command first.
pub const STATUS_YIELD: [StatusSlot; 7] = [
    StatusSlot::Answer,
    StatusSlot::Command,
    StatusSlot::Autosave,
    StatusSlot::Selection,
    StatusSlot::Path,
    StatusSlot::Render,
    StatusSlot::Units,
];

/// How wide an item of the status bar wants to be and how narrow it may
/// be elided to (as wide as it wants for one that is not elided), in
/// points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusFit {
    pub slot: StatusSlot,
    pub natural: f32,
    pub least: f32,
}

/// An item of the status bar as it is drawn.
struct StatusItem {
    slot: StatusSlot,
    text: String,
    tint: egui::Color32,
    elide: bool,
}

impl StatusItem {
    fn fixed(slot: StatusSlot, text: String, tint: egui::Color32) -> Self {
        Self {
            slot,
            text,
            tint,
            elide: false,
        }
    }

    fn elided(slot: StatusSlot, text: String, tint: egui::Color32) -> Self {
        Self {
            slot,
            text,
            tint,
            elide: true,
        }
    }
}

/// The widths the items of the status bar get in `available` points, with
/// `gap` between two that are shown: each as wide as it wants when they
/// all fit; otherwise, in the order of [`STATUS_YIELD`], items are elided
/// down to their least width until they fit, and when that is not enough
/// the first in the order are left out (`None`), the others elided only
/// as far as still needed. Never more than `available` in all.
pub fn fit_status(available: f32, gap: f32, items: &[StatusFit]) -> Vec<Option<f32>> {
    let clean = |w: f32| if w.is_finite() { w.max(0.0) } else { 0.0 };
    let available = if available.is_finite() {
        available.max(0.0)
    } else {
        0.0
    };
    let gap = if gap.is_finite() {
        gap.max(0.0)
    } else {
        available + 1.0
    };
    let total = |widths: &[Option<f32>]| {
        let shown: Vec<f32> = widths.iter().flatten().copied().collect();
        shown.iter().sum::<f32>() + gap * shown.len().saturating_sub(1) as f32
    };
    // The first `dropped` of the order left out, the rest elided as far as
    // needed, in the order.
    for dropped in 0..=STATUS_YIELD.len() {
        let gone = STATUS_YIELD.get(..dropped).unwrap_or(&STATUS_YIELD);
        let mut widths: Vec<Option<f32>> = items
            .iter()
            .map(|i| (!gone.contains(&i.slot)).then(|| clean(i.natural)))
            .collect();
        let mut over = total(&widths) - available;
        for slot in STATUS_YIELD {
            if over <= 0.0 {
                break;
            }
            for (item, width) in items.iter().zip(widths.iter_mut()) {
                if item.slot != slot {
                    continue;
                }
                if let Some(w) = width {
                    let least = clean(item.least).min(*w);
                    let shrunk = (*w - over).max(least);
                    over -= *w - shrunk;
                    *w = shrunk;
                }
            }
        }
        if total(&widths) <= available + 1e-3 {
            return widths;
        }
    }
    vec![None; items.len()]
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

/// A clock for timing a render (in the browser the page's own, since
/// `std::time::Instant` has none there).
fn stopwatch() -> impl FnOnce() -> Option<f64> {
    #[cfg(not(target_arch = "wasm32"))]
    let start = std::time::Instant::now();
    #[cfg(target_arch = "wasm32")]
    let start = web_time::Instant::now();
    move || Some(start.elapsed().as_secs_f64() * 1000.0)
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
            report: report::ReportState::default(),
            copy_hit: None,
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
            overlay_density: None,
            scheduler: Scheduler::new(Mode::Inline),
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
            arrived: Vec::new(),
            renderer: BodyRenderer::new(choice),
            shown_report: None,
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
            fields: number_field::FieldMemory::default(),
            typing: None,
            length_unit: "",
            section_hits: Vec::new(),
            show_hits: Vec::new(),
            camera_hits: Vec::new(),
            controls_layout: None,
            status_hits: Vec::new(),
            zoom_anchor: None,
            select: selection::Selecting::default(),
            timeline: timeline::Timeline::default(),
            player: player::Player::default(),
            orbiting: false,
            tree_panel: None,
            cause_hits: Vec::new(),
            section_cache: None,
            section_lost: None,
            framed: Bbox3::EMPTY,
            themed: false,
            quit: false,
            quit_discards: false,
            quitting: None,
            rows: Vec::new(),
            assertion_rows: Vec::new(),
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
            views: BTreeMap::new(),
            viewing: None,
            fly: None,
            made_selection: None,
            tree_scroll: None,
            tab_switched: false,
            tree_row_hits: Vec::new(),
            tree_view: None,
            params: params::ParamForm::default(),
            help: help::HelpMemory::default(),
            recent: recent::RecentMemory::default(),
            start: start::StartMemory::default(),
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
    /// reason kept for [`Self::renderer_report`]. In the browser there are
    /// no threads: the CPU renderer then stands by on the page's own
    /// thread.
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
        if let Some(direct) = &mut self.direct {
            direct.without_see_through();
        }
        self.renderer.without_gpu_see_through();
    }

    /// As on a device that cannot draw a cut-open picture: from now on a
    /// frame with a section is the CPU renderer's. For tests of that
    /// fallback.
    pub fn without_gpu_section(&mut self) {
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

    /// The density of the edges per tile the direct path measured for the
    /// picture it shows (columns, rows, means): for tests.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn direct_edge_density(&mut self) -> Option<Result<crate::direct::EdgeTiles, String>> {
        let shown = self.direct_shown;
        self.direct
            .as_mut()
            .filter(|_| shown)
            .map(Direct::read_edge_density)
    }

    /// True when the viewport is drawn directly on the window's device.
    pub fn draws_directly(&self) -> bool {
        self.direct.is_some()
    }

    /// Stops drawing on the window's device, for `why`: from now on the
    /// viewport is rendered as without a device of the window's. For a
    /// host that learns of a failure of the device the shell cannot see
    /// itself (the browser reports errors of WebGPU later, to a handler
    /// of the page's).
    pub fn leave_gpu(&mut self, why: String) {
        if self.direct.take().is_some() {
            self.direct_fallback = Some(why);
            self.direct_shown = false;
            self.shown_report = None;
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

    /// The document's assertions as of the last [`Self::sync`], each with
    /// how it stands: shown under the features of the tree.
    pub fn assertion_rows(&self) -> &[craft3d_engine::AssertionState] {
        &self.assertion_rows
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

    /// Stops the application without asking, dropping the unsaved
    /// changes and their autosaves (`ui.quit`).
    pub fn request_quit(&mut self) {
        self.quit = true;
        self.quit_discards = true;
    }

    /// Stops the application once every document was saved
    /// (`ui.quit {save: "save"}`): nothing unsaved is left to keep.
    pub fn request_quit_saved(&mut self) {
        self.quit = true;
    }

    /// True when the application was asked to stop without saving: the
    /// autosaves of what was not saved go too. Otherwise a document that
    /// still has unsaved changes when the application ends (one changed
    /// while it was closing) keeps its autosave, as after a crash.
    pub fn quit_discards(&self) -> bool {
        self.quit_discards
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
        let mut ids: Vec<u32> = self.selection_faces().collect();
        if let (Some(face), Some(scene)) = (&self.state.highlight, scene)
            && let Ok(found) = scene.faces_named(face.feature.as_deref(), &face.face)
        {
            ids.extend(found.ids);
        }
        if ids.is_empty() {
            return Highlight::NONE;
        }
        Highlight::new(self.tokens.face_highlight, ids).0
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
        .with_selection(self.selection_marks())
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

    /// Where the last frame drew the name of each row of the feature list
    /// (the feature's id and the rectangle, in points); rows scrolled out
    /// of view are listed too, outside [`Self::tree_view`].
    pub fn tree_row_hits(&self) -> &[(String, egui::Rect)] {
        &self.tree_row_hits
    }

    /// The part of the window the feature list shows its rows in, once it
    /// was drawn with rows: its right edge is where the scroll bar lies.
    pub fn tree_view(&self) -> Option<egui::Rect> {
        self.tree_view
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
            // else the face. Lit by pointing; a click that selects it
            // says so after (`click_select`).
            self.highlight_set_elsewhere();
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
        self.follow_active_tab(session);
        self.sync_timeline(session);
        self.sync_player(session);
        let scene = self.sync_scene(session);
        self.mark_scrubbed_rows();
        self.assertion_rows = session.assertion_states();
        self.sync_params(session, scene.as_ref());
        self.sync_tabs(session);
        self.note_recent(session);
        self.shown_status = session.document_status();
        self.shown_scene.clone_from(&scene);
        self.sync_report(session);
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
            self.forget_selection();
            self.made_selection = None;
            self.framed = Bbox3::EMPTY;
            // The section stays as it is set: it is resolved again when
            // there is a document to cut.
            self.section_cache = None;
            return None;
        };
        self.resolve_section(session, &scene);
        self.sync_selection(session, &scene);
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
                hidden: f.hidden,
                made_selection: self.made_selection.as_deref() == Some(f.id.as_str()),
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
        self.params_frame(ui.ctx());
        let (scene, status) = match session.as_deref_mut() {
            Some(session) => {
                self.length_unit = session.length_unit().unwrap_or("");
                (self.sync(session), session.document_status())
            }
            None => (self.shown_scene.clone(), self.shown_status.clone()),
        };
        self.file_shortcuts(ui, status.is_some());
        self.tab_shortcuts(ui);
        self.help_keys(ui);
        // The next step of the history player, when it is time.
        self.advance_player(ui.ctx());
        let tokens = self.tokens;
        egui::Panel::top("craft3d-menu")
            .resizable(false)
            .show_separator_line(false)
            .frame(chrome_frame(tokens.menu_fill))
            .show(ui, |ui| {
                ui.style_mut().visuals = tokens.chrome_visuals();
                self.menu_bar(ui, status.is_some());
            });
        if self.file_action.is_some() || self.doc_action.is_some() {
            // The host does it between frames: have one come.
            ui.ctx().request_repaint();
        }
        if self.state.panels.recovery && !self.recovery.found.is_empty() {
            let clicked = egui::Panel::top("craft3d-recovery")
                .resizable(false)
                .show(ui, |ui| self.recovery_bar(ui))
                .inner;
            if let (Some(action), Some(session)) = (clicked, session.as_deref_mut()) {
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
            egui::Panel::top("craft3d-tabs")
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(color(tokens.menu_fill))
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
                .show_separator_line(false)
                .frame(chrome_frame(tokens.status_fill))
                .show(ui, |ui| {
                    ui.style_mut().visuals = tokens.chrome_visuals();
                    self.status_bar(ui, status.as_ref());
                });
        }
        self.cause_hits.clear();
        // Put away while the welcome shows (`start.rs`).
        if self.state.panels.tree && !self.welcome() {
            // No margin on the right: the list's scroll bar lies against
            // the panel's edge, the split it is resized by.
            let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin {
                left: side_inset(&tokens),
                right: 0,
                top: 2,
                bottom: 2,
            });
            let panel = egui::Panel::left("craft3d-tree")
                .resizable(true)
                .default_size(260.0)
                .frame(frame)
                .show(ui, |ui| self.side_panel(ui, status.is_some()));
            self.tree_panel = Some(panel.response.rect);
        } else {
            self.tree_panel = None;
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, scene.as_ref()));
        // A parameter's value that broke the model, and the way back.
        self.trouble_banner(ui);
        // Space, the arrows, Home and End over the viewport or the list.
        self.player_keys(ui.ctx());
        // A number typed into a field of the viewport, once Enter was
        // pressed: evaluated with the document's variables, shown next
        // frame (or its error is).
        if let Some(session) = session.as_deref() {
            let before = self.typing.clone();
            self.resolve_typing(session);
            self.resolve_param_typing(Some(session));
            if self.command_waiting() {
                ui.ctx().request_repaint();
            }
            if self.typing != before {
                ui.ctx().request_repaint();
            }
        }
        // What the timeline and the eyes asked for: the scene of a new
        // step is built where the window's scenes are, not here.
        if let Some(session) = session.as_deref_mut()
            && self.apply_timeline_requests(session)
        {
            ui.ctx().request_repaint();
        }
        // What the mouse selected, now that the session is here.
        if let Some(session) = session
            && self.apply_selection_requests(session)
        {
            self.sync(session);
            ui.ctx().request_repaint();
        }
        self.closing_dialog(ui);
        self.help_windows(ui);
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
            FileAction::Import,
            FileAction::Export,
        ] {
            let pressed = action
                .shortcut()
                .is_some_and(|shortcut| ui.input_mut(|i| i.consume_shortcut(&shortcut)));
            if pressed && (has_document || !action.needs_document()) {
                self.file_action = Some(action);
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, has_document: bool) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                for action in FileAction::ALL {
                    if action.starts_group() {
                        ui.separator();
                    }
                    let shortcut = action
                        .shortcut()
                        .map(|shortcut| ui.ctx().format_shortcut(&shortcut))
                        .unwrap_or_default();
                    let button = egui::Button::new(action.label()).shortcut_text(shortcut);
                    let enabled = (has_document || !action.needs_document())
                        && self.file_action_available(action);
                    if ui.add_enabled(enabled, button).clicked() {
                        self.file_action = Some(action);
                        ui.close();
                    }
                }
                self.recent_menu(ui);
                ui.separator();
                self.menu_documents(ui);
            });
            self.help_menu(ui);
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

    /// The status bar: units, the document's path, when it was autosaved,
    /// the last command and what it answered on the left; the renderer's
    /// numbers on the right. When the bar is too narrow the items give way
    /// in a fixed order ([`fit_status`]): first they are elided (with the
    /// whole text as a tooltip), then left out, so no two overlap.
    fn status_bar(&mut self, ui: &mut egui::Ui, status: Option<&DocumentStatus>) {
        let tokens = self.tokens;
        let dim = color(tokens.chrome_text_dim);
        let mut items: Vec<StatusItem> = Vec::new();
        match status {
            Some(status) => {
                let units = format!("{} · {}", status.length_unit, status.angle_unit);
                items.push(StatusItem::fixed(
                    StatusSlot::Units,
                    units,
                    color(tokens.chrome_text),
                ));
                let name = status.path.as_deref().unwrap_or("untitled");
                let dirty = if status.dirty { " *" } else { "" };
                items.push(StatusItem::elided(
                    StatusSlot::Path,
                    format!("{name}{dirty}"),
                    color(tokens.chrome_text),
                ));
            }
            None => items.push(StatusItem::fixed(
                StatusSlot::Units,
                "no document".to_string(),
                dim,
            )),
        }
        if let Some((text, gone)) = self.selection_status() {
            let tint = if gone {
                color(tokens.chrome_failed)
            } else {
                color(tokens.chrome_text)
            };
            items.push(StatusItem::elided(StatusSlot::Selection, text, tint));
        }
        if let Some((text, problem)) = autosave_text(&self.recovery, epoch_now()) {
            let tint = if problem {
                color(tokens.chrome_failed)
            } else {
                dim
            };
            items.push(StatusItem::elided(StatusSlot::Autosave, text, tint));
            if !problem {
                // "3 min ago" grows old without anything else to draw.
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(30));
            }
        }
        if let Some(report) = &self.last_command {
            let tint = if report.ok {
                tokens.chrome_ok
            } else {
                tokens.chrome_failed
            };
            items.push(StatusItem::fixed(
                StatusSlot::Command,
                report.label.clone(),
                color(tint),
            ));
            items.push(StatusItem::elided(
                StatusSlot::Answer,
                report.text.clone(),
                dim,
            ));
        }
        if let Some(stats) = self.stats {
            let time = stats
                .milliseconds
                .map_or_else(String::new, |ms| format!(" · {ms:.0} ms"));
            // The quick first picture is shown while the display mesh is
            // built: said here, unobtrusively.
            let refining = if self.shown_scene.as_ref().is_some_and(|s| s.preview) {
                "refining · "
            } else {
                ""
            };
            let text = format!(
                "{refining}{} {}×{} · {} triangles{time}",
                stats.renderer.name().to_uppercase(),
                stats.width,
                stats.height,
                stats.triangles
            );
            items.push(StatusItem::fixed(StatusSlot::Render, text, dim));
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let painter = ui.painter().clone();
        let measure = |text: &str| {
            painter
                .layout_no_wrap(text.to_string(), font.clone(), dim)
                .size()
                .x
        };
        let fits: Vec<StatusFit> = items
            .iter()
            .map(|item| {
                let natural = measure(&item.text).ceil();
                StatusFit {
                    slot: item.slot,
                    natural,
                    least: if item.elide {
                        natural.min(tokens.status_least)
                    } else {
                        natural
                    },
                }
            })
            .collect();
        let available = ui.available_width();
        // "Copy details" after the answer of a command that failed
        // (`shell/report.rs`): its room is kept out of what the items get.
        let copy = self
            .last_command
            .as_ref()
            .is_some_and(|r| !r.ok)
            .then(|| self.copy_label());
        let copy_width = copy.map_or(0.0, |label| {
            measure(label) + 2.0 * ui.spacing().button_padding.x + tokens.status_gap
        });
        let widths = fit_status((available - copy_width).max(0.0), tokens.status_gap, &fits);
        let height = ui.spacing().interact_size.y;
        let mut hits = Vec::new();
        let mut copy_clicked = false;
        let mut copy_rect = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut used = 0.0;
            let mut first = true;
            let mut copy_button = |ui: &mut egui::Ui, used: &mut f32| {
                if let Some(label) = copy {
                    ui.add_space(tokens.status_gap);
                    let response = ui
                        .add(egui::Button::new(label))
                        .on_hover_text(report::COPY_TOOLTIP);
                    copy_clicked |= response.clicked();
                    copy_rect = Some(response.rect);
                    *used += copy_width;
                }
            };
            let mut copy_shown = false;
            for (item, width) in items.iter().zip(&widths) {
                let Some(width) = *width else { continue };
                if item.slot == StatusSlot::Render && !copy_shown {
                    copy_button(ui, &mut used);
                    copy_shown = true;
                }
                if item.slot == StatusSlot::Render {
                    // Right-aligned: the room left goes before it.
                    let room = (available - used - width).max(0.0);
                    ui.add_space(room);
                    used += room;
                } else if !first {
                    // A divider in the middle of the gap, except between a
                    // command and its answer.
                    let (gap, _) = ui.allocate_exact_size(
                        egui::vec2(tokens.status_gap, height),
                        egui::Sense::hover(),
                    );
                    if item.slot != StatusSlot::Answer {
                        let x = gap.center().x;
                        ui.painter().line_segment(
                            [
                                egui::pos2(x, gap.top() + height * tokens.divider_inset),
                                egui::pos2(x, gap.bottom() - height * tokens.divider_inset),
                            ],
                            egui::Stroke::new(
                                tokens.divider_width,
                                color_alpha(tokens.chrome_text_dim, tokens.divider_opacity),
                            ),
                        );
                    }
                    used += tokens.status_gap;
                }
                first = false;
                let label =
                    egui::Label::new(egui::RichText::new(&item.text).color(item.tint)).truncate();
                let response = ui
                    .allocate_ui_with_layout(
                        egui::vec2(width, height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.set_width(width);
                            ui.add(label)
                        },
                    )
                    .inner;
                hits.push((item.slot, response.rect));
                if width + 0.5 < measure(&item.text) {
                    // The whole text, where the bar is too narrow for it.
                    response.on_hover_text(&item.text);
                }
                used += width;
                if item.slot == StatusSlot::Answer && !copy_shown {
                    copy_button(ui, &mut used);
                    copy_shown = true;
                }
            }
            if !copy_shown {
                copy_button(ui, &mut used);
            }
        });
        self.status_hits = hits;
        self.copy_hit = copy_rect;
        if copy_clicked {
            self.copy_error_report(ui.ctx());
        }
        self.error_details(ui.ctx());
    }

    /// Where the status bar's "Copy details" was drawn in the last frame:
    /// `None` when the last command did not fail.
    pub fn copy_details_rect(&self) -> Option<egui::Rect> {
        self.copy_hit
    }

    /// Where the items of the status bar were drawn in the last frame.
    pub fn status_hits(&self) -> &[(StatusSlot, egui::Rect)] {
        &self.status_hits
    }

    fn tree(&mut self, ui: &mut egui::Ui, has_document: bool) {
        let tokens = self.tokens;
        let dim = color(tokens.text_dim);
        // The timeline at the foot of the panel, under the list.
        egui::Panel::bottom("craft3d-timeline")
            .resizable(false)
            .show(ui, |ui| {
                // Clear of the split, as the rows above.
                ui.set_max_width((ui.available_width() - f32::from(side_inset(&tokens))).max(40.0));
                self.timeline_ui(ui);
            });
        if self.rows.is_empty() {
            let hint = if has_document {
                "No features yet."
            } else {
                "No document."
            };
            ui.colored_label(dim, hint);
            if has_document {
                assertions_heading(ui, &tokens, self.assertion_rows.is_empty());
            }
            return;
        }
        let mut clicked = None;
        let mut double = None;
        let mut followed = None;
        let mut eye_clicked = None;
        let in_use = self.jig_in_use();
        let mut hits = Vec::new();
        let scroll_to = self.tree_scroll.take();
        let mut rows_drawn = Vec::new();
        // F over the list as over the viewport: to the selection.
        let fit = ui.rect_contains_pointer(ui.max_rect())
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(plain_f);
        // Each document keeps where its list was scrolled to. The bar
        // lies against the panel's edge: the area takes the whole width
        // and the rows keep clear of the bar.
        let list = egui::ScrollArea::vertical()
            .id_salt(("craft3d-tree-rows", self.viewing))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: side_inset(&tokens),
                        top: 0,
                        bottom: 0,
                    })
                    .show(ui, |ui| {
                        for row in &self.rows {
                            let tint = match row.state {
                                FeatureState::Ok => tokens.ok,
                                FeatureState::Failed => tokens.failed,
                                FeatureState::Suppressed | FeatureState::RolledBack => {
                                    tokens.inactive
                                }
                            };
                            let mark = tree_mark(row.state);
                            let mut name = egui::RichText::new(&row.name);
                            match row.state {
                                FeatureState::Ok => {}
                                FeatureState::Failed => name = name.color(color(tokens.failed)),
                                FeatureState::Suppressed => {
                                    name = name.color(color(tokens.inactive)).strikethrough();
                                }
                                FeatureState::RolledBack => {
                                    name = name.color(color(tokens.inactive)).italics()
                                }
                            }
                            let hover = match (&row.error, row.state) {
                                (Some(error), _) => error.clone(),
                                (None, FeatureState::Suppressed) => {
                                    "Suppressed: not built, and what is made from it fails."
                                        .to_string()
                                }
                                (None, FeatureState::RolledBack)
                                    if row.status == timeline::AFTER_SCRUB =>
                                {
                                    "After the step the timeline shows: not shown now.".to_string()
                                }
                                (None, FeatureState::RolledBack) => {
                                    "Rolled back: after the rollback marker, not built.".to_string()
                                }
                                (None, _) => format!("{} · {}", row.kind, row.status),
                            };
                            ui.horizontal(|ui| {
                                ui.colored_label(color(tint), mark).on_hover_text(&hover);
                                let label = ui
                                    .selectable_label(row.selected || row.made_selection, name)
                                    .on_hover_text(&hover);
                                rows_drawn.push((row.id.clone(), label.rect));
                                // The second click of a double click is a click
                                // too: the double click wins.
                                if label.double_clicked() {
                                    double = Some(row.id.clone());
                                } else if label.clicked() {
                                    clicked = Some(row.id.clone());
                                }
                                if scroll_to.as_deref() == Some(row.id.as_str()) {
                                    // Only as far as it takes (a row in view
                                    // stays), and at once: the list follows a
                                    // click in the viewport, it does not glide.
                                    label.scroll_to_me_animation(
                                        None,
                                        egui::style::ScrollAnimation::none(),
                                    );
                                }
                                ui.colored_label(dim, format!("{} · {}", row.kind, row.status));
                                // A Jig in use is looked at, not changed.
                                if !in_use && timeline::eye(ui, row.hidden, &tokens).clicked() {
                                    eye_clicked = Some((row.id.clone(), row.hidden));
                                }
                            });
                            if let Some((cause, cause_name)) = &row.caused_by {
                                ui.indent(("craft3d-cause", &row.id), |ui| {
                                    ui.horizontal(|ui| {
                                        ui.colored_label(dim, "caused by");
                                        let link = ui
                                            .link(
                                                egui::RichText::new(cause_name)
                                                    .color(color(tokens.failed)),
                                            )
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
                        assertions_heading(ui, &tokens, self.assertion_rows.is_empty());
                        let mut pending = false;
                        for row in &self.assertion_rows {
                            use craft3d_engine::AssertionStatus as A;
                            // Passed: built; failed or in error: failed; not proven
                            // either way: needs attention; being measured: inactive.
                            let (state, tint) = match row.status {
                                A::Passed => (FeatureState::Ok, tokens.ok),
                                A::Failed | A::Error => (FeatureState::Failed, tokens.failed),
                                A::NotProven => (FeatureState::Failed, tokens.attention),
                                A::Pending => (FeatureState::RolledBack, tokens.inactive),
                            };
                            pending |= row.status == A::Pending;
                            let mut name = egui::RichText::new(&row.name);
                            if row.status != A::Passed {
                                name = name.color(color(tint));
                            }
                            let status = match &row.measured {
                                Some(measured) => format!("{} · {}", row.measure, measured),
                                None => format!("{} · {}", row.measure, row.status.name()),
                            };
                            ui.horizontal(|ui| {
                                ui.colored_label(color(tint), tree_mark(state))
                                    .on_hover_text(&row.message);
                                ui.label(name).on_hover_text(&row.message);
                                ui.colored_label(dim, status);
                            });
                            if matches!(row.status, A::Failed | A::Error | A::NotProven) {
                                ui.indent(("craft3d-assertion", &row.name), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&row.message)
                                                .color(color(tint))
                                                .small(),
                                        )
                                        .wrap(),
                                    );
                                });
                            }
                        }
                        // Measured on another thread: look again soon.
                        if pending {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(250));
                        }
                    });
            });
        self.cause_hits = hits;
        if let Some((id, hidden)) = eye_clicked {
            self.toggle_visible(id, hidden);
            ui.ctx().request_repaint();
        }
        self.tree_row_hits = rows_drawn;
        self.tree_view = Some(list.inner_rect);
        // A double click on a row selects that feature (whatever its
        // first click did) and flies to what it made.
        if let Some(id) = double {
            self.state.selected_feature = Some(id.clone());
            self.queue_select(serde_json::json!({"items": [{"kind": "feature", "feature": id}]}));
            self.request_fly(views::FlyTo::Feature(id));
            return;
        }
        if fit {
            let now = ui.input(|i| i.time);
            self.fit_key(self.shown_scene.clone().as_deref(), now);
        }
        // A click on a cause selects that feature (it does not toggle: the
        // point is to get there).
        if let Some(id) = followed {
            self.state.selected_feature = Some(id.clone());
            self.queue_select(serde_json::json!({"items": [{"kind": "feature", "feature": id}]}));
            return;
        }
        // A click on a row selects that feature; Shift+click adds it or
        // takes it out; clicking the selected row again clears the
        // selection. What is selected is view state: nothing in the
        // document changes.
        if let Some(id) = clicked {
            let shift = ui.input(|i| i.modifiers.shift);
            let only = self.state.selected_feature.as_ref() == Some(&id)
                && self.selection().items.len() <= 1;
            let request = if shift {
                serde_json::json!({"items": [{"kind": "feature", "feature": id}], "mode": "toggle"})
            } else if only {
                serde_json::json!({"mode": "clear"})
            } else {
                serde_json::json!({"items": [{"kind": "feature", "feature": id}]})
            };
            self.state.selected_feature = if only && !shift { None } else { Some(id) };
            self.queue_select(request);
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

        self.start_requested_fly(scene, ui.input(|i| i.time));
        let turning = self.advance_turn(ui, scene);
        let moving =
            self.pointer(ui, &response, rect, scene) || resized || turning || self.orbiting;
        let [width, height] = self.size;
        let pixel = |at: egui::Pos2| {
            let across = |from: f32, to: f32, at: f32, pixels: u32| {
                f64::from(at - from) / f64::from((to - from).max(f32::MIN_POSITIVE))
                    * f64::from(pixels)
            };
            (
                across(rect.left(), rect.right(), at.x, width),
                across(rect.top(), rect.bottom(), at.y, height),
            )
        };
        // A click that was not a drag points at what is under it, and
        // selects it (`selection.rs`).
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            let (x, y) = pixel(at);
            let keys = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
            let double = response.double_clicked();
            // A picture that cannot be made has nothing to point at; the
            // viewport says why already.
            if let Ok(found) = self.pick(scene, x, y, None, None, true) {
                self.click_select(scene, x, y, &found, (keys.0, keys.1, double));
            }
            ui.ctx().request_repaint();
        }
        // A right click lists every face under it.
        if response.secondary_clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            let (x, y) = pixel(at);
            let shift = ui.input(|i| i.modifiers.shift);
            self.open_other_menu(scene, x, y, at, shift);
            ui.ctx().request_repaint();
        }
        if response.contains_pointer()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(help::CLEAR_KEY))
        {
            self.escape_selection();
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
            selection: self.selection_drawn(),
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
            let [w, h] = self.overlay_size;
            let pixels_per_point = if rect.width() > 0.0 {
                f64::from(w) / f64::from(rect.width())
            } else {
                1.0
            };
            let key = (pixels_per_point.to_bits(), tokens);
            if self.overlay_density.as_ref().is_none_or(|(k, _)| *k != key) {
                let density =
                    viewport::EdgeDensity::new(&self.overlay, w, h, &tokens, pixels_per_point);
                self.overlay_density = Some((key, density));
            }
            if let Some((_, density)) = &self.overlay_density {
                paint_overlay(
                    &painter,
                    rect,
                    self.overlay_size,
                    &self.overlay,
                    &tokens,
                    density,
                );
            }
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
        let start = self.welcome();
        match (scene, self.problem()) {
            (None, None) if self.work.is_some() => hint("No document."),
            (Some(scene), _) if scene.bbox.is_empty() => hint("Empty document."),
            _ => {}
        }
        // The caption of a narrated demo, then the history player's step
        // under it: plates at the top, in the middle.
        let mut top = inner.top();
        let captions = [
            (!self.state.notice.is_empty()).then(|| (self.state.notice.clone(), 17.0)),
            self.player_caption().map(|text| (text, 15.0)),
        ];
        for (text, size) in captions.into_iter().flatten() {
            let galley = painter.layout_no_wrap(
                text,
                egui::FontId::proportional(size),
                color(tokens.notice_text),
            );
            let text_size = galley.size();
            let center = egui::pos2(rect.center().x, top + text_size.y);
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
            top = plate.bottom() + inset * 0.5;
        }

        if let Some(work) = &self.work {
            self.work_plate(ui, &painter, inner, work);
        }

        // The box being dragged, and the "select other" list.
        self.paint_selecting(ui);
        // The view cube, the triad and the controls over the viewport;
        // the start screen instead while there is no document.
        if start {
            self.controls_layout = None;
        } else {
            self.viewport_controls(ui, rect, scene);
        }
        if let Some(problem) = self.problem().map(str::to_string) {
            self.problem_text(&painter, rect, &problem);
        }
        // No document: New, Open, the examples, the recent files and how
        // to connect an agent (`start.rs`).
        if start {
            self.start_screen(ui, rect);
        } else {
            self.no_start_screen();
        }
    }

    /// What went wrong, in the bottom left corner of the viewport `rect`:
    /// right of the triad and left of the display bar, wrapped to fit
    /// there; above the bar when there is no room beside it.
    fn problem_text(&self, painter: &egui::Painter, rect: egui::Rect, problem: &str) {
        let tokens = self.tokens;
        let layout = self.controls_layout.unwrap_or_default();
        let font = egui::FontId::proportional(tokens.problem_font);
        let natural = painter
            .layout_no_wrap(problem.to_string(), font.clone(), color(tokens.failed))
            .size()
            .x;
        let Some([left, width, bottom]) = layout::problem_place(rect, &layout, &tokens, natural)
        else {
            return;
        };
        let galley = painter.layout(problem.to_string(), font, color(tokens.failed), width);
        let top = bottom - galley.size().y;
        painter.galley(egui::pos2(left, top), galley, color(tokens.failed));
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
        let camera = view_cube::between(&turn.from, &turn.to, elapsed / turn.seconds);
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
        // Ctrl and a drag draw a box to select with, not a turn.
        if self.box_drag(ui, response) {
            ui.ctx().request_repaint();
            return false;
        }
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
            // Keys typed into a field are not shortcuts, and the wheel over
            // the focused number field steps it rather than zooming; over
            // any other field it zooms.
            let typing = ui.ctx().egui_wants_keyboard_input();
            let fields = self
                .controls_layout
                .map(|l| match self.fields.focus {
                    Some("see-through") => [l.see_through, None],
                    Some("offset") => [l.offset, None],
                    _ => [None, None],
                })
                .unwrap_or_default();
            let (scroll, pinch, fit, pointer) = ui.input(|i| {
                let pointer = i.pointer.hover_pos();
                let on_field =
                    pointer.is_some_and(|at| fields.iter().flatten().any(|f| f.contains(at)));
                (
                    if on_field {
                        0.0
                    } else {
                        f64::from(i.smooth_scroll_delta().y)
                    },
                    f64::from(i.zoom_delta()),
                    plain_f(i) && !typing,
                    pointer,
                )
            });
            // A pinch (or Ctrl and the wheel) as the turn of the wheel
            // that scales the distance alike.
            let pinch = if pinch > 0.0 && pinch.is_finite() && pinch != 1.0 {
                pinch.ln() / crate::camera::ZOOM_PER_POINT
            } else {
                0.0
            };
            let scroll = scroll + pinch;
            if scroll != 0.0 && scroll.is_finite() {
                match pointer.and_then(|at| self.zoom_point(at, rect, scene)) {
                    Some(point) => {
                        self.state.camera.zoom_about(scroll, point);
                        self.zoom_anchor = pointer.map(|pointer| ZoomAnchor {
                            pointer,
                            camera: self.state.camera,
                            point,
                        });
                    }
                    None => self.state.camera.zoom(scroll),
                }
                moving = true;
            }
            if fit {
                self.fit_key(scene, ui.input(|i| i.time));
            }
        }
        moving
    }

    /// The point a zoom at `pointer` (in points, in the viewport `rect`)
    /// keeps in place: the point of the model the viewport shows there
    /// (the same answer a click gets, cut open by the section when it is
    /// on), or else the point of the plane through the target square to
    /// the view. The same as the last zoom's while neither the pointer nor
    /// the camera moved since.
    fn zoom_point(
        &self,
        pointer: egui::Pos2,
        rect: egui::Rect,
        scene: Option<&Scene>,
    ) -> Option<craft3d_math::Vec3> {
        if let Some(anchor) = self.zoom_anchor
            && anchor.pointer == pointer
            && anchor.camera == self.state.camera
        {
            return Some(anchor.point);
        }
        if !rect.contains(pointer) {
            return None;
        }
        let [width, height] = self.size;
        let across = |from: f32, to: f32, at: f32, pixels: u32| {
            f64::from(at - from) / f64::from((to - from).max(f32::MIN_POSITIVE)) * f64::from(pixels)
        };
        let x = across(rect.left(), rect.right(), pointer.x, width);
        let y = across(rect.top(), rect.bottom(), pointer.y, height);
        let camera = self.state.camera;
        let section = self.section();
        let hit = scene.and_then(|scene| {
            let (face, cap) = scene
                .pick_cut(&camera.to_render(), width, height, x, y, section.as_ref())
                .ok()?;
            face.map(|face| face.point).or(cap.map(|cap| cap.point))
        });
        hit.filter(|p| p.is_finite())
            .or_else(|| camera.target_plane_point(x, y, f64::from(width), f64::from(height)))
    }

    /// The direct path's texture, when the picture on screen is its.
    fn direct_texture(&self) -> Option<egui::TextureId> {
        self.direct
            .as_ref()
            .filter(|_| self.direct_shown)
            .and_then(Direct::texture_id)
    }

    /// Draws `key` on the window's device, when the shell has it. True
    /// when the picture on screen is now the direct path's; false when
    /// there is no direct path, or this frame has to be made another way.
    fn direct_frame(&mut self, scene: Option<&Scene>, key: &FrameKey) -> bool {
        let cut = self.section_cut().filter(|_| key.section.is_some());
        let chosen = self.selection_marks().clone();
        let Some(direct) = &mut self.direct else {
            return false;
        };
        direct.set_highlight(key.highlight);
        direct.set_opacity(key.opacity);
        direct.set_marks(
            marks_of(
                scene,
                key.highlight_edge.as_ref(),
                key.highlight_vertex.as_ref(),
                cut,
                key.datums,
                key.sketches,
            )
            .with_selection(&chosen),
        );
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
        let chosen = self.selection_marks().clone();
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
            )
            .with_selection(&chosen),
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
        self.overlay_density = None;
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

/// What the feature panel says under "Assertions" while the document has
/// none: what they are for, and how to get one.
pub const NO_ASSERTIONS_HINT: &str = "No checks yet. Ask an AI agent, e.g. \u{201c}check that the wall is at least 2 mm\u{201d}, or use assert.add.";

/// The "Assertions" heading of the feature panel, always there; with the
/// hint while there are none.
fn assertions_heading(ui: &mut egui::Ui, tokens: &Tokens, empty: bool) {
    ui.add_space(tokens.inset * 0.5);
    ui.label(egui::RichText::new("Assertions").strong());
    ui.separator();
    if empty {
        ui.add(
            egui::Label::new(
                egui::RichText::new(NO_ASSERTIONS_HINT)
                    .color(color(tokens.text_dim))
                    .small(),
            )
            .wrap(),
        );
    }
}

/// The frame of a chrome bar (the menu bar, the status bar): `fill`, with
/// the margins of egui's own side panels.
fn chrome_frame(fill: crate::theme::Rgb) -> egui::Frame {
    egui::Frame::new()
        .fill(color(fill))
        .inner_margin(egui::Margin::symmetric(8, 2))
}

fn paint_overlay(
    painter: &egui::Painter,
    rect: egui::Rect,
    size: [u32; 2],
    overlay: &Overlay,
    tokens: &Tokens,
    density: &viewport::EdgeDensity,
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
    // The edges fade where they crowd (`viewport::EdgeDensity`, measured
    // in the image's pixels at its pixels per point); the painter draws a
    // line in one colour, so an edge is drawn in pieces of at most half a
    // tile, each faded as at its middle.
    let pixels_per_point = if rect.width() > 0.0 {
        f64::from(w) / f64::from(rect.width())
    } else {
        1.0
    };
    let piece = f64::from(viewport::tile_pixels(tokens, pixels_per_point)) * 0.5;
    let mut colors = viewport::EdgeColorCache::default();
    for line in &overlay.lines {
        let alpha = f64::from(viewport::opacity(line.hidden, tokens));
        let ink = match line.edge {
            Some(edge) => colors.get(edge.body, tokens).pick(edge.silhouette),
            None => line.color(tokens),
        };
        let stroke = |fade: f64| {
            let opacity = (alpha * fade).round().clamp(0.0, 255.0) as u8;
            egui::Stroke::new(line.width(tokens), color_alpha(ink, opacity))
        };
        let Some(edge) = line.edge else {
            painter.line_segment([place(line.from), place(line.to)], stroke(1.0));
            continue;
        };
        let long = viewport::fade_short(edge.extent, tokens, pixels_per_point);
        let length = (line.to[0] - line.from[0]).hypot(line.to[1] - line.from[1]);
        let pieces = (length / piece).ceil();
        let pieces = if pieces.is_finite() {
            pieces.clamp(1.0, 4096.0) as usize
        } else {
            1
        };
        let at = |t: f64| {
            [
                line.from[0] + (line.to[0] - line.from[0]) * t,
                line.from[1] + (line.to[1] - line.from[1]) * t,
            ]
        };
        for k in 0..pieces {
            let (t0, t1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
            let middle = at(0.5 * (t0 + t1));
            let fade = viewport::edge_fade(long, density.at(middle[0], middle[1]), tokens);
            painter.line_segment([place(at(t0)), place(at(t1))], stroke(fade));
        }
    }
    for point in &overlay.points {
        let tint = color_alpha(
            point.kind.color(tokens),
            viewport::opacity(point.hidden, tokens),
        );
        painter.circle_filled(place(point.at), point.kind.dot_radius(tokens), tint);
    }
}
