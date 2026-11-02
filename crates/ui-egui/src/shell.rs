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
use craft3d_math::Portable;
use craft3d_render::{Highlight, Section};
use std::collections::BTreeMap;
use std::sync::Arc;

pub mod agents;
mod bake_hint;
mod calm;
mod controls;
mod dialogs;
mod edit;
pub mod help;
pub(crate) mod icons;
mod joints;
mod layout;
pub mod look_panel;
mod menus;
pub mod number_field;
mod params;
mod player;
mod popover;
pub mod recent;
mod report;
mod selection;
pub mod sidepanel;
pub mod start;
mod tabs;
mod timeline;
mod views;

pub use controls::ControlsLayout;
pub use dialogs::{ErrorDialog, ExportAsk, ExportRequest, error_title, export_verb};
pub use edit::{EditAction, EditRequest, MAX_EDIT_QUEUE, run_edit};
pub use help::{External, Shortcut, ShortcutGroup, ShortcutInput, help_shortcut, shortcuts};
pub use layout::{ControlWidths, controls_layout, triad_side};
pub use params::{
    DEBOUNCE, Faults, NOTE_SECONDS, Pacer, Param, ParamAsk, ParamKind, ParamList, Settled,
    ShellCommand, Verdict, reported_faults, settle, verdict,
};
pub use popover::{cube_out_of_the_way, popover_place, split_chip};
pub use recent::{MAX_RECENT, RecentEntry, RecentFiles};
pub use start::{AgentSetup, EXAMPLES, Example, ExampleSection, McpClient, run_example};
pub use tabs::{
    CLOSE_MARK, CloseAnswer, DocAction, TabPart, UNSAVED_MARK, close_shortcut, doc_tabs,
    next_shortcut, previous_shortcut, tab_glyph, tab_name,
};
pub(crate) use views::FlyTo;
pub use views::{FLY_SECONDS, MIN_FLY_CONTEXT, ZOOM_TOOLTIP, zoomable};

/// The viewport's size before the window has laid it out, in pixels: what
/// `ui.screenshot` renders in a host without a window.
pub const DEFAULT_VIEWPORT: [u32; 2] = [960, 640];

/// While the camera moves the image is rendered this much smaller per side
/// and without supersampling, so orbiting stays fluid on the CPU renderer.
/// (None of the rules below apply to the direct GPU path of the window,
/// [`crate::direct`]: there every frame is the viewport's full size, with
/// multisampling, and nothing is read back.)
pub const MOVING_DIVISOR: u32 = 2;
/// What the status bar says when the viewport leaves the window's graphics
/// device after an error the device reported ([`Shell::leave_gpu`]).
pub const LEFT_GPU_NOTICE: &str = "The graphics card reported an error; the viewport now draws without it (slower). Reload the page to try again.";

/// How long after the camera last moved the studio look stays light where
/// the host asked for that ([`Shell::interacting_light`], the browser), in
/// seconds of input time: long enough to bridge the gaps between the
/// wheel's notches and a drag's pauses, short enough that the full look
/// follows as soon as the hand rests.
pub const STILL_DELAY: f64 = 0.3;
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
    /// A new part (`file.new`).
    New,
    /// A new assembly (`assembly.new`).
    NewAssembly,
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
    pub const ALL: [FileAction; 9] = [
        FileAction::New,
        FileAction::NewAssembly,
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
            FileAction::New => "New Part",
            FileAction::NewAssembly => "New Assembly",
            FileAction::Open => "Open…",
            FileAction::Save => "Save",
            FileAction::SaveAs => "Save As…",
            FileAction::Import => "Import…",
            FileAction::Export => "Export…",
            FileAction::ExportJig => "Export as Jig…",
            FileAction::OpenForEditing => "Open for editing",
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
            FileAction::NewAssembly => (command | egui::Modifiers::SHIFT, egui::Key::N),
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
        matches!(
            self,
            FileAction::Open | FileAction::Save | FileAction::Import
        )
    }

    /// The line icon beside it in the File menu: the kind of document a
    /// New makes (the plan's middle way).
    pub fn glyph(self) -> Option<crate::kit::Glyph> {
        match self {
            FileAction::New => Some(crate::kit::Glyph::Part),
            FileAction::NewAssembly => Some(crate::kit::Glyph::Assembly),
            _ => None,
        }
    }
}

/// Where a caption plate `half` wide on either side is centred in the
/// viewport `rect`: in its middle, unless it would reach the camera
/// `plate` beside it, then in the middle of the room left of the plate
/// (and never past the viewport's left margin).
pub(crate) fn caption_x(
    rect: egui::Rect,
    plate: Option<egui::Rect>,
    half: f32,
    margin: f32,
    gap: f32,
) -> f32 {
    let middle = rect.center().x;
    match plate {
        Some(plate) if middle + half + gap > plate.left() => {
            let left = rect.left() + margin;
            let right = plate.left() - gap;
            ((left + right) * 0.5).max(left + half)
        }
        _ => middle,
    }
}

/// The side panel's width for the person's `wanted` width in a window
/// `window` points wide: within the tokens' range and share of the
/// window, the default when `wanted` is no number.
pub(crate) fn tree_width(tokens: &Tokens, wanted: f32, window: f32) -> f32 {
    let wanted = if wanted.is_finite() {
        wanted
    } else {
        tokens.tree_width
    };
    let most = tokens
        .tree_max_width
        .min(window * tokens.tree_max_share)
        .max(tokens.tree_min_width);
    let most = if most.is_finite() {
        most
    } else {
        tokens.tree_min_width
    };
    wanted.clamp(tokens.tree_min_width, most)
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
    /// Something of it can be shown and flown to (a double click, the
    /// row's zoom icon): [`zoomable`](crate::shell::zoomable).
    pub zoomable: bool,
    /// The condition that switches it on (`feature.when`), as written.
    pub when: Option<String>,
    /// What it passes on and leaves out, in a sentence
    /// (`SceneFeature::passes`).
    pub passes: Option<String>,
    /// In an assembly's tree: the instance the row is, when it is the
    /// assembly's own (its eye and its Open act on it).
    pub instance: Option<String>,
    /// The agent that made it (the document's provenance, `madeBy`), or
    /// last changed it; `None` for the person's: its row carries the ✦.
    pub made_by: Option<String>,
    /// In an assembly's tree: how deep in subassemblies the row is.
    pub depth: usize,
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
        // Switched off as by suppression: the same mark; the row says
        // `when` beside it.
        FeatureState::Suppressed | FeatureState::Inactive => suppressed,
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

/// What the status bar shows of things that change fast, held so that
/// it does not flicker (`shell/calm.rs`).
#[derive(Clone, Debug, Default)]
struct Calm {
    /// The work indicator.
    work: calm::WorkIndicator,
    /// What it said in the frame last drawn (`None`: not shown).
    work_shown: Option<String>,
    /// The last command and its answer, and the renderer's numbers.
    /// Held as one unit (`ok`, label and answer joined), so a new label
    /// never shows with the old answer and the tint is the shown text's.
    command: calm::Held,
    render: calm::Held,
    /// Their widths while only their numbers change.
    command_width: calm::Steady,
    answer_width: calm::Steady,
    render_width: calm::Steady,
    /// How many times the texts of the status bar changed, for tests.
    changes: u64,
    /// The texts of the last frame, to count the changes by.
    last: Vec<(StatusSlot, String)>,
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
    /// How the bodies' edges are drawn ([`Shell::edges_drawn`]).
    edges: EdgesDrawn,
    /// What the traces drawn depend on ([`Shell::traces_key`]): a trace
    /// made, cleared or grown is another picture.
    traces: (u64, Option<u64>),
    /// How the bodies are lit.
    look: crate::state::LookState,
    /// The surface analysis drawn, by when it was made.
    analysis: u64,
    /// The studio look's environment is prepared (always true for the
    /// plain look): the frame drawn plain while it was not is redrawn.
    look_ready: bool,
    /// Drawn lighter while a motion plays ([`Shell::playing_light`]): the
    /// picture of a moment drawn so is drawn again at the full quality
    /// once it stops.
    light: bool,
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

/// What a surface analysis was made from: the scene's revision and
/// sampling, the state and the theme's colours.
#[derive(Clone, Debug, PartialEq)]
struct AnalysisSource {
    revision: u64,
    tolerance: (f64, Option<f64>),
    state: crate::state::AnalysisState,
    colors: craft3d_render::analysis::AnalysisColors,
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
    /// The studio look, or `None` for the plain one.
    studio: Option<craft3d_gpu::Studio>,
    /// The surface analysis, or `None`.
    analysis: Option<Arc<viewport::ViewAnalysis>>,
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
        renderer.set_studio(self.studio.clone());
        renderer.set_analysis(self.analysis.clone());
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
    // native clock: `web_time`'s in the browser (the `use` above).
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|since| since.as_secs())
}

pub struct Shell {
    pub state: UiState,
    /// The side panel's memory: what it read, what it asked for, what is
    /// being typed (`shell/sidepanel`).
    side: sidepanel::SideMemory,
    /// A frame-exact recording keeps the clock (`record.rs`): nothing
    /// measured on the wall clock is drawn (a picture's milliseconds in
    /// the status bar), so the same requests give the same window.
    frame_exact: bool,
    /// A picture of the window is being taken (`ui.screenshot`): like a
    /// frame-exact recording, it shows the exact moment of a motion at
    /// full quality ([`Self::exact_pictures`]).
    pub(crate) capturing: bool,
    /// The window is being recorded (`ui.record`, in real time or frame
    /// by frame): a playing motion is then drawn at the full quality, not
    /// lighter ([`Self::playing_light`]).
    recording: bool,
    /// The studio look is drawn lighter while the camera moves
    /// ([`Self::interacting_light`]): set by the host, on in the browser,
    /// whose one thread and integrated GPUs cannot draw the full look at
    /// the rate a turn of the camera asks for.
    light_while_moving: bool,
    /// The input time of the last frame in which the camera moved (a drag,
    /// the wheel, a pinch, a fly to a view, the player's orbit), while the
    /// look is drawn light for it.
    moved_at: Option<f64>,
    /// The camera moved within [`STILL_DELAY`] of this frame
    /// ([`Self::interacting_light`]).
    interacting: bool,
    /// While [`Self::interacting_light`], the direct path draws the
    /// viewport this many times smaller per side, stretched over it (1:
    /// at its full size). Set by the host with
    /// [`Self::set_light_while_moving`].
    moving_divisor: u32,
    pub tokens: Tokens,
    pub last_command: Option<CommandReport>,
    /// The work indicator and the texts of the status bar, held so they
    /// do not flicker (`shell/calm.rs`).
    calm: Calm,
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
    /// Why the viewport left the window's device ([`Shell::leave_gpu`]).
    left_gpu: Option<String>,
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
    /// Where the studio look's options were drawn ([`look_panel::LookHits`]).
    look_hits: look_panel::LookHits,
    /// Where the Analysis popover's panel was drawn (`panel`).
    /// Where the carets of the Studio and Analysis split buttons were
    /// drawn (they open and close the popovers).
    look_caret: Option<egui::Rect>,
    analysis_caret: Option<egui::Rect>,
    analysis_hits: Vec<(String, egui::Rect)>,
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
    /// The window is narrower than `Tokens::narrow_width`: the narrow
    /// layout (a drawer for the side panel). The tokens and egui's
    /// spacing without touch sizes, kept while those are in use.
    narrow: bool,
    wide: Option<(Tokens, egui::style::Spacing)>,
    /// Touch sizes are in use; a finger has been seen on the screen, and
    /// when last (a tap's release frame has no touch any more).
    touch_sized: bool,
    touch_seen: bool,
    last_touch: f64,
    /// When and where the last tap was: the first of a double tap.
    last_tap: Option<(f64, egui::Pos2)>,
    /// Where the drawer's button was drawn, in the narrow layout.
    drawer_button: Option<egui::Rect>,
    /// Two fingers were on the viewport: the finger left on it does not
    /// turn the camera until it is lifted.
    touch_gesture: bool,
    /// The player drew its `t` this frame, and in the last one: the
    /// caption over the viewport leaves it to the player then.
    player_shown: bool,
    /// Where the player's row drew its parts (`player.rs`).
    player_hits: Vec<(&'static str, egui::Rect)>,
    /// Who made each feature (an agent's name), for the document and
    /// revision it was read for: the ✦ of the feature list.
    provenance: (
        Option<(Option<u64>, u64)>,
        std::collections::BTreeMap<String, String>,
    ),
    player_shown_last: bool,
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
    /// Whether the panels have the dark appearance now: the state's
    /// `appearance` resolved against the system's preference each frame.
    dark: bool,
    /// The system's preference as the last frame saw it (`None`: it said
    /// none), so `ui.get` resolves a change of the state at once.
    system_theme: Option<egui::Theme>,
    /// The desktop's light or dark setting as the host read it, for when
    /// the windowing library reports none.
    desktop_theme: Option<egui::Theme>,
    /// The interface's fonts: Inter and JetBrains Mono once they are here
    /// (`crate::fonts`).
    fonts: crate::fonts::UiFonts,
    quit: bool,
    /// The application was asked to stop without saving (`ui.quit`, or
    /// `ui.quit {save: "discard"}`): the autosaves go too.
    quit_discards: bool,
    /// Closing the window, one question per document with unsaved
    /// changes ([`Self::request_window_close`]).
    quitting: Option<tabs::Quitting>,
    rows: Vec<TreeRow>,
    /// The active assembly's outline, when the document is one.
    outline: Option<craft3d_engine::AssemblyOutline>,
    /// Where the last frame drew each instance row's eye and Open button.
    instance_hits: Vec<(String, &'static str, egui::Rect)>,
    /// The joints of the Parameters tab: the slider dragged now (its
    /// mate and value), and where each slider was drawn.
    joints: joints::Joints,
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
    /// The surface analysis drawn ([`Self::sync_analysis`]), what it was
    /// made from, how often it was made (for the frame key), why it could
    /// not be, and the mode the Analysis toggle turns on again.
    pub(crate) analysis_view: Option<Arc<viewport::ViewAnalysis>>,
    analysis_made_from: Option<AnalysisSource>,
    analysis_serial: u64,
    pub(crate) analysis_problem: Option<String>,
    pub(crate) analysis_last: String,
    /// Millimetres per length unit of the shown document (the lens of
    /// the studio look's depth of field).
    length_mm: f64,
    /// The narrow layout's drawer was sliding in or out in the last frame.
    drawer_sliding: bool,
    shown_status: Option<DocumentStatus>,
    /// The work being done on the host's engine thread, if any.
    work: Option<Work>,
    /// The open documents as of the last sync: the row of tabs.
    tabs: Vec<crate::state::DocTab>,
    /// The open documents (by `DocumentStatus::id`) seen to be assemblies
    /// while they were the active one: their tabs' icon.
    assembly_docs: std::collections::BTreeSet<u64>,
    /// Where the menus were drawn (`shell/menus.rs`).
    menu_hits: menus::MenuHits,
    /// The question before closing a document, while it is asked.
    closing: Option<crate::state::Closing>,
    /// What a tab, the File menu or a shortcut asked for, until the host
    /// takes it.
    doc_action: Option<DocAction>,
    /// Undo or Redo asked by the Edit menu or a shortcut, until the host
    /// takes it.
    edit_actions: std::collections::VecDeque<EditRequest>,
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
    /// The features the shown scene shows something of, for the scene
    /// (revision and sampling) it was worked out for (`views.rs`).
    shown_features: Option<views::ShownFeatures>,
    /// Where the last frame drew the name of each row of the feature
    /// list, and the part of the window the list shows its rows in.
    tree_row_hits: Vec<(String, egui::Rect)>,
    /// Where the last frame drew the zoom icon of a row (`views.rs`).
    tree_zoom_hits: Vec<(String, egui::Rect)>,
    /// The error and export dialogs (`dialogs.rs`).
    dialogs: dialogs::Dialogs,
    tree_view: Option<egui::Rect>,
    /// The Parameters tab: its list, the field being changed, the commit
    /// waiting for the host (`shell/params.rs`).
    params: params::ParamForm,
    /// Whether AI agents may drive the window (`shell/agents.rs`).
    agents: agents::AgentsMemory,
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
    /// What is being done while work runs (`Opening welding-table…
    /// 41/82`, `Checking 13 assertions… 5/13`), on the right before the
    /// renderer's numbers, always as wide (`Tokens::status_work_width`).
    Work,
    /// The renderer's numbers, on the right.
    Render,
}

/// The order in which the items give way: each is first elided to its
/// least width in this order, then left out in this order. The units go
/// last, the answer to the last command first.
pub const STATUS_YIELD: [StatusSlot; 8] = [
    StatusSlot::Answer,
    StatusSlot::Command,
    StatusSlot::Autosave,
    StatusSlot::Selection,
    StatusSlot::Path,
    StatusSlot::Work,
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
    /// As wide as this whatever the text (elided within), when set.
    width: Option<f32>,
}

impl StatusItem {
    fn fixed(slot: StatusSlot, text: String, tint: egui::Color32) -> Self {
        Self {
            slot,
            text,
            tint,
            elide: false,
            width: None,
        }
    }

    fn elided(slot: StatusSlot, text: String, tint: egui::Color32) -> Self {
        Self {
            slot,
            text,
            tint,
            elide: true,
            width: None,
        }
    }

    /// The items at the right end of the bar.
    fn on_the_right(&self) -> bool {
        matches!(self.slot, StatusSlot::Work | StatusSlot::Render)
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
        (FeatureState::Inactive, _, _) => "inactive".to_string(),
        (FeatureState::RolledBack, _, _) => "rolled back".to_string(),
    }
}

/// A clock for timing a render (in the browser the page's own, since
/// `std::time::Instant` has none there).
/// Who made each feature of the active document, by its id: the agent
/// that made it, or else the one that last changed it (the document's
/// provenance, as `agent.log` gives it).
fn provenance(session: &Session) -> std::collections::BTreeMap<String, String> {
    session
        .agent_log_json(0)
        .get("features")
        .and_then(serde_json::Value::as_array)
        .map(|features| {
            features
                .iter()
                .filter_map(|f| {
                    let id = f.get("id")?.as_str()?.to_string();
                    let by = f
                        .get("madeBy")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| f.get("changedBy").and_then(serde_json::Value::as_str))?;
                    Some((id, by.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn stopwatch() -> impl FnOnce() -> Option<f64> {
    #[cfg(not(target_arch = "wasm32"))]
    // native clock: the line above is cfg(not(target_arch = "wasm32")).
    let start = std::time::Instant::now();
    #[cfg(target_arch = "wasm32")]
    let start = web_time::Instant::now();
    move || Some(start.elapsed().as_secs_f64() * 1000.0)
}

impl Shell {
    /// How the bodies' edges are drawn now: `show.edges`, and the look's
    /// `edges`.
    fn edges_drawn(&self) -> EdgesDrawn {
        EdgesDrawn {
            shown: self.state.show.edges,
            mode: self.state.look.edge_mode(),
        }
    }

    /// The studio look the viewport draws with, or `None` for the plain
    /// look, and while the environment is being prepared (on a thread of
    /// its own on the desktop, fetched by the page in the browser): a frame
    /// never waits for it.
    pub fn studio(&self) -> Option<craft3d_gpu::Studio> {
        self.studio_with(craft3d_engine::look::VIEWPORT_ACCUMULATE)
    }

    /// [`Self::studio`] for a picture drawn once rather than refined
    /// while the camera stands still (the paths besides the window's own
    /// device, `ui.screenshot`): one frame unless the look gives
    /// `accumulate`.
    pub fn studio_once(&self) -> Option<craft3d_gpu::Studio> {
        self.studio_with(1)
    }

    fn studio_with(&self, frames: u32) -> Option<craft3d_gpu::Studio> {
        if !self.state.look.is_studio() {
            return None;
        }
        let spec = self.state.look.spec().ok()?;
        let env = spec.prepared_if_ready()?;
        let mut studio = self.studio_of(&spec, env, frames);
        // While a motion plays the window draws lighter; still pictures,
        // captures and recordings never come through here lightened
        // ([`Self::playing_light`]).
        if self.playing_light() || self.interacting_light() {
            studio.look = player::light_look(studio.look);
        }
        Some(studio)
    }

    /// The studio look of `spec` lit by `env`, for the shown scene: the
    /// lens focused and scaled for it, `frames` frames unless the look
    /// says.
    fn studio_of(
        &self,
        spec: &craft3d_engine::look::LookSpec,
        env: std::sync::Arc<craft3d_render::studio::Prepared>,
        frames: u32,
    ) -> craft3d_gpu::Studio {
        let model = self
            .shown_scene
            .as_ref()
            .map_or(craft3d_math::Bbox3::EMPTY, |scene| scene.bbox);
        craft3d_gpu::Studio {
            env,
            look: spec.resolved(&model, self.length_mm, &self.focus_at(), frames),
        }
    }

    /// Whether the viewport's picture is blurred by the studio look's
    /// lens: the studio look drawn (its environment prepared, else the
    /// plain look is shown meanwhile), a lens, no analysis (drawn in the
    /// plain look), and one that blurs something ([`Self::lens_focus`]:
    /// not an orthographic view, not a focus behind the camera).
    pub fn lens_on(&self) -> bool {
        self.look_ready() && self.lens_focus().is_some_and(|lens| lens.no_blur.is_none())
    }

    /// The points the lens may focus on: the last click and the centre
    /// of the selection ([`craft3d_engine::look::FocusAt`]).
    pub fn focus_at(&self) -> craft3d_engine::look::FocusAt {
        craft3d_engine::look::FocusAt {
            picked: self.state.picked.as_ref().and_then(|p| p.point),
            selection: self.select.focus,
            selected: !self.select.view.items.is_empty() || !self.select.view.hidden.is_empty(),
            gone: !self.select.view.dropped.is_empty(),
        }
    }

    /// Prepares the studio look's environment now, waiting for it (once
    /// per environment, on the desktop): for a picture of the window,
    /// which must show the look asked for and not the plain one drawn
    /// meanwhile. Nothing to do for the plain look.
    pub fn prepare_look(&self) -> Result<(), String> {
        if !self.state.look.is_studio() {
            return Ok(());
        }
        self.state.look.spec()?.prepared().map(|_| ())
    }

    /// Whether what the window shows is still on its way at egui's time
    /// `now`: the studio look's environment being prepared (the plain
    /// look is drawn meanwhile), or something moving into place
    /// ([`Self::animating`]). A picture of the window waits for both.
    pub fn look_settling(&self, ctx: &egui::Context, now: f64) -> bool {
        !self.look_ready() || self.animating(ctx, now)
    }

    /// Whether the window's own animations are under way at egui's time
    /// `now`: a popover fading in, the narrow layout's drawer sliding in
    /// or out (as of the last frame).
    pub fn animating(&self, ctx: &egui::Context, now: f64) -> bool {
        self.popover_fading(ctx, now) || self.drawer_sliding
    }

    /// Whether the look can be drawn as asked now: the plain look, or the
    /// studio look with its environment prepared.
    fn look_ready(&self) -> bool {
        !self.state.look.is_studio()
            || craft3d_engine::look::environment_ready(&self.state.look.environment)
    }

    /// [`Self::studio_once`], waiting for the environment to be prepared:
    /// for a picture asked for (`ui.screenshot`), which must show the
    /// look.
    pub fn studio_now(&self) -> Option<craft3d_gpu::Studio> {
        if !self.state.look.is_studio() {
            return None;
        }
        let spec = self.state.look.spec().ok()?;
        let env = spec.prepared().ok()??;
        Some(self.studio_of(&spec, env, 1))
    }

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
            side: sidepanel::SideMemory::default(),
            frame_exact: false,
            capturing: false,
            recording: false,
            light_while_moving: false,
            moved_at: None,
            interacting: false,
            moving_divisor: 1,
            tokens: Tokens::default(),
            last_command: None,
            calm: Calm::default(),
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
            left_gpu: None,
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
            look_hits: Vec::new(),
            look_caret: None,
            analysis_caret: None,
            analysis_hits: Vec::new(),
            camera_hits: Vec::new(),
            controls_layout: None,
            status_hits: Vec::new(),
            zoom_anchor: None,
            select: selection::Selecting::default(),
            timeline: timeline::Timeline::default(),
            player: player::Player::default(),
            orbiting: false,
            tree_panel: None,
            narrow: false,
            wide: None,
            touch_sized: false,
            touch_seen: false,
            last_touch: f64::NEG_INFINITY,
            last_tap: None,
            drawer_button: None,
            touch_gesture: false,
            player_shown: false,
            player_hits: Vec::new(),
            provenance: (None, std::collections::BTreeMap::new()),
            player_shown_last: false,
            cause_hits: Vec::new(),
            section_cache: None,
            section_lost: None,
            framed: Bbox3::EMPTY,
            themed: false,
            dark: false,
            system_theme: None,
            desktop_theme: None,
            fonts: crate::fonts::UiFonts::default(),
            quit: false,
            quit_discards: false,
            quitting: None,
            rows: Vec::new(),
            outline: None,
            instance_hits: Vec::new(),
            joints: joints::Joints::default(),
            assertion_rows: Vec::new(),
            file_action: None,
            problem: None,
            render_problem: None,
            shown_scene: None,
            analysis_view: None,
            analysis_made_from: None,
            analysis_serial: 0,
            analysis_problem: None,
            analysis_last: "zebra".to_string(),
            length_mm: 1.0,
            drawer_sliding: false,
            shown_status: None,
            work: None,
            tabs: Vec::new(),
            assembly_docs: std::collections::BTreeSet::new(),
            menu_hits: menus::MenuHits::default(),
            closing: None,
            doc_action: None,
            edit_actions: std::collections::VecDeque::new(),
            tab_hits: Vec::new(),
            closing_hits: Vec::new(),
            tab_scrolled: None,
            views: BTreeMap::new(),
            viewing: None,
            fly: None,
            made_selection: None,
            tree_scroll: None,
            tab_switched: false,
            shown_features: None,
            tree_row_hits: Vec::new(),
            tree_zoom_hits: Vec::new(),
            dialogs: dialogs::Dialogs::default(),
            tree_view: None,
            params: params::ParamForm::default(),
            agents: agents::AgentsMemory::default(),
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

    /// A frame-exact recording keeps the clock (`record.rs`): while it
    /// does, nothing measured on the wall clock is drawn.
    pub fn set_frame_exact(&mut self, exact: bool) {
        self.frame_exact = exact;
    }

    /// A picture of the window is being taken (`ui.screenshot`) while
    /// `capturing` is true ([`Self::exact_pictures`]).
    pub fn set_capturing(&mut self, capturing: bool) {
        self.capturing = capturing;
    }

    /// The window is recorded (`ui.record`, in real time or frame by
    /// frame) while `recording` is true: a playing motion is drawn at the
    /// full quality then ([`Self::playing_light`]).
    pub fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
    }

    /// Whether the window is recorded ([`Self::set_recording`]).
    pub fn recording(&self) -> bool {
        self.recording
    }

    /// True while the viewport draws lighter because a motion plays
    /// (`timeline.playQuality` auto): the studio look in one frame,
    /// without its depth of field and its most costly screen-space passes
    /// (`player::light_look`). Never for what is recorded
    /// or captured (a picture of the window, `ui.record` in real time or
    /// frame by frame): those are drawn at the full quality, as a motion
    /// that stands still is (docs/design/motion.md › *The player*).
    pub fn playing_light(&self) -> bool {
        self.state.timeline.playing
            && self.state.timeline.play_quality == crate::state::PlayQuality::Auto
            && self.playing_motion()
            && !self.exact_pictures()
            && !self.recording
    }

    /// Draw the studio look lighter while the camera moves
    /// ([`Self::interacting_light`]); the browser's host turns it on.
    pub fn set_light_while_moving(&mut self, light: bool) {
        self.light_while_moving = light;
        if !light {
            self.moving_divisor = 1;
            self.interacting = false;
        }
    }

    /// While the camera moves ([`Self::interacting_light`]) the direct
    /// path draws the viewport `divisor` times smaller per side (1 to 4),
    /// stretched over it, its lines thinner by as much so they look the
    /// same; at rest it is drawn at its full size again. Only with
    /// [`Self::set_light_while_moving`] on.
    pub fn set_moving_divisor(&mut self, divisor: u32) {
        self.moving_divisor = divisor.clamp(1, 4);
    }

    /// Whether the studio look is drawn lighter while the camera moves
    /// ([`Self::set_light_while_moving`]).
    pub fn light_while_moving(&self) -> bool {
        self.light_while_moving
    }

    /// True while the viewport draws lighter because the camera moves (a
    /// drag, the wheel, a pinch, a fly to a view, the player's orbit) or
    /// moved less than [`STILL_DELAY`] ago, where the host asked for it
    /// ([`Self::set_light_while_moving`], the browser): the same light look
    /// as a playing motion (`player::light_look`: one frame, no depth of
    /// field, no occlusion, no screen-space reflections, no contact or
    /// specular shadows). Once the camera rests the full look is drawn and
    /// settles as before. Never for what is recorded or captured.
    pub fn interacting_light(&self) -> bool {
        self.interacting && !self.exact_pictures() && !self.recording
    }

    /// Notes whether the camera `moving` in this frame, at input time
    /// `now`, and returns how long the light look still lasts when the
    /// camera stands still, to look again then.
    fn note_interaction(&mut self, moving: bool, now: f64) -> Option<f64> {
        if !self.light_while_moving {
            self.interacting = false;
            self.moved_at = None;
            return None;
        }
        if moving && now.is_finite() {
            self.moved_at = Some(now);
        }
        let left = self.moved_at.and_then(|at| {
            let since = now - at;
            // A clock that went back (another context) ends it too.
            (since.is_finite() && (0.0..STILL_DELAY).contains(&since))
                .then_some(STILL_DELAY - since)
        });
        if left.is_none() {
            self.moved_at = None;
        }
        self.interacting = left.is_some();
        left.filter(|_| !moving)
    }

    /// True while what the viewport draws is recorded or captured (a
    /// frame-exact recording, a picture of the window): a playing motion
    /// then shows each moment exactly (`Session::set_motion_live` off, the
    /// player asks with `shapes: "exact"`) and at the full quality, never
    /// the provisional moment or the quick first picture a play shows on
    /// screen, so the frames do not depend on timing
    /// (docs/design/motion.md › *The player*).
    pub fn exact_pictures(&self) -> bool {
        self.frame_exact || self.capturing
    }

    /// Whether the viewport, drawn directly, is still averaging the frames
    /// of a still picture (`Look::accumulate`): a picture of the window
    /// taken now would be half done.
    pub fn converging(&self) -> bool {
        self.direct_shown && self.direct.as_ref().is_some_and(Direct::converging)
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
    ///
    /// When the viewport was drawn on that device, the status bar says so
    /// once (the error's first line with it), and `ui.get`'s
    /// `renderer.leftGpu` keeps why: a silent, slower fallback hid a
    /// shader the browser refused (docs/design/rendering.md › *The
    /// browser*, 2026-11-02).
    pub fn leave_gpu(&mut self, why: String) {
        if self.direct.take().is_some() {
            let first = why.lines().next().unwrap_or_default();
            self.last_command = Some(CommandReport {
                label: "graphics".to_string(),
                ok: false,
                text: format!("{LEFT_GPU_NOTICE} ({first})"),
            });
            self.left_gpu = Some(why.clone());
            self.direct_fallback = Some(why);
            self.direct_shown = false;
            self.shown_report = None;
        }
    }

    /// Why the viewport left the window's graphics device
    /// ([`Self::leave_gpu`]), when it did.
    pub fn left_gpu(&self) -> Option<&str> {
        self.left_gpu.as_deref()
    }

    /// The quality the viewport asks the engine for: the display rule
    /// with the state's `quality` ([`crate::state::DisplayQuality`]). The
    /// scene the shell shows, and `ui.screenshot` renders, is
    /// `session.scene_with(shell.scene_quality())`.
    pub fn scene_quality(&self) -> SceneQuality {
        // While a motion plays, each moment is shown as the quick first
        // picture: one scene per moment instead of two, no display meshes
        // made and kept for moments that are gone the next frame
        // (docs/design/motion.md › *The player*). Paused, the moment
        // shown is refined as any picture is.
        if self.state.timeline.playing && self.playing_motion() && !self.exact_pictures() {
            return SceneQuality::Preview;
        }
        self.state.quality.scene_quality()
    }

    /// True when sharp images are made on a thread of their own.
    pub fn renders_in_background(&self) -> bool {
        self.scheduler.mode() == Mode::Background
    }

    /// True while a sharp image is being made on its thread, or has been
    /// made and is not on screen yet: a frame drawn now is not the
    /// finished picture (a frame-exact recording waits for it).
    pub fn sharp_pending(&self) -> bool {
        self.renders_in_background() && self.scheduler.pending().is_some()
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
        // native clock: `wait_for_sharp` is cfg(not(target_arch = "wasm32")).
        let deadline = std::time::Instant::now().checked_add(timeout);
        loop {
            let left = deadline.map_or(timeout, |deadline| {
                // native clock: as above.
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

    /// The active assembly's outline as the tree lists it, when the
    /// document is an assembly.
    pub fn assembly_outline(&self) -> Option<&craft3d_engine::AssemblyOutline> {
        self.outline.as_ref()
    }

    /// Where the last frame drew each instance row's controls: the
    /// instance, `eye` or `open`, and the rectangle.
    pub fn instance_hits(&self) -> &[(String, &'static str, egui::Rect)] {
        &self.instance_hits
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
    ///
    /// While the studio look's lens blurs the picture ([`Self::lens_on`])
    /// the selected faces keep their own colour, so the part in focus
    /// shows its sharpness: the selection is then said by its outline,
    /// drawn over the picture. A click's own light on what it selected
    /// goes too. Still lit: selected faces that get no outline (a full
    /// sphere, a face bounded by a seam only, an outline past its cap),
    /// the entry of the "select other" list under the pointer, and a face
    /// `ui.highlight` points at that is not selected.
    pub fn face_highlight(&self, scene: Option<&Scene>) -> Highlight {
        let lens = self.lens_on();
        let mut ids: Vec<u32> = if lens {
            self.select
                .bare
                .iter()
                .copied()
                .chain(self.select.hover)
                .collect()
        } else {
            self.selection_faces().collect()
        };
        let pointed = if lens && self.click_lit() {
            None
        } else {
            self.state.highlight.as_ref()
        };
        if let (Some(face), Some(scene)) = (pointed, scene)
            && let Ok(found) = scene.faces_named(face.feature.as_deref(), &face.face)
        {
            // Under the lens a face pointed at that is selected is said
            // by the selection's outline.
            let selected = &self.select.faces;
            ids.extend(
                found
                    .ids
                    .into_iter()
                    .filter(|id| !lens || selected.binary_search(id).is_err()),
            );
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
            self.edges_drawn(),
        )
        .with_selection(self.selection_marks())
        .with(viewport::Marks {
            traces: self.drawn_traces(),
            ..viewport::Marks::default()
        })
        .with(self.comb_marks())
    }

    /// What the viewport draws over the picture of `key`: the marks of
    /// [`Self::marks`] as the key has them. The one list every path that
    /// draws the window's picture takes (the direct path and the
    /// renderer's images), so none of them leaves a kind of mark out.
    fn frame_marks(&self, scene: Option<&Scene>, key: &FrameKey) -> viewport::Marks {
        marks_of(
            scene,
            key.highlight_edge.as_ref(),
            key.highlight_vertex.as_ref(),
            self.section_cut().filter(|_| key.section.is_some()),
            key.datums,
            key.sketches,
            key.edges,
        )
        .with_selection(self.selection_marks())
        .with(viewport::Marks {
            traces: self.drawn_traces(),
            ..viewport::Marks::default()
        })
        .with(self.comb_marks())
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
        // What `file.export` to DXF or SVG draws as the view seen.
        session.set_window_camera(Some(self.state.camera.to_render()));
        self.sync_timeline(session);
        self.sync_player(session);
        let scene = self.sync_scene(session);
        // Again once the camera followed the model to this scene: the view
        // seen is the one this frame shows, not the last frame's.
        session.set_window_camera(Some(self.state.camera.to_render()));
        self.sync_analysis(session, scene.as_ref());
        self.mark_scrubbed_rows();
        self.mark_zoomable(scene.as_deref());
        self.assertion_rows = session.assertion_states();
        self.sync_params(session, scene.as_ref());
        self.sync_side(session, scene.as_deref());
        self.sync_tabs(session);
        self.note_recent(session);
        self.shown_status = session.document_status();
        self.shown_scene.clone_from(&scene);
        self.length_mm = session.length_mm();
        self.sync_report(session);
        scene
    }

    /// Makes the surface analysis of the shown scene when its state, the
    /// scene or the theme changed: the curvature map's values from the
    /// exact surfaces and the combs (`craft3d_engine::analysis`). Off, or
    /// without a scene, there is none.
    fn sync_analysis(&mut self, session: &mut Session, scene: Option<&Arc<Scene>>) {
        let (Some(scene), true) = (scene, self.state.analysis.is_on()) else {
            if self.analysis_view.take().is_some() {
                self.analysis_serial = self.analysis_serial.wrapping_add(1);
            }
            self.analysis_made_from = None;
            self.analysis_problem = None;
            return;
        };
        let source = AnalysisSource {
            revision: scene.revision,
            tolerance: scene.sampling(),
            state: self.state.analysis.clone(),
            colors: self.tokens.analysis_colors(),
        };
        if self.analysis_made_from.as_ref() == Some(&source) {
            return;
        }
        self.analysis_made_from = Some(source);
        self.analysis_serial = self.analysis_serial.wrapping_add(1);
        match self.make_analysis(session, scene) {
            Ok(view) => {
                self.analysis_view = Some(Arc::new(view));
                self.analysis_problem = None;
            }
            Err(why) => {
                self.analysis_view = None;
                self.analysis_problem = Some(why);
            }
        }
    }

    fn make_analysis(
        &self,
        session: &mut Session,
        scene: &Scene,
    ) -> Result<viewport::ViewAnalysis, String> {
        use craft3d_render::analysis::AnalysisMode;
        let spec = self.state.analysis.spec()?;
        let mut values = Vec::new();
        let mut mesh = None;
        let mut range = spec.range.unwrap_or([-1.0, 1.0]);
        if spec.mode == Some(AnalysisMode::Curvature) {
            // On the mesh refined where the curvature changes fast, as
            // `view.render` draws the map.
            let (refined, map) =
                craft3d_engine::analysis::refined_map(session, scene, spec.curvature)
                    .map_err(|e| e.to_string())?;
            values = map;
            mesh = Some(Arc::new(refined));
            range = spec
                .range
                .unwrap_or_else(|| craft3d_engine::analysis::auto_range(&values, spec.curvature));
        }
        let combs = match &spec.combs {
            Some(c) => {
                craft3d_engine::analysis::combs(session, scene, c).map_err(|e| e.to_string())?
            }
            None => Vec::new(),
        };
        let unit = craft3d_engine::analysis::length_unit(session);
        Ok(viewport::ViewAnalysis {
            analysis: spec.analysis(range, self.tokens.analysis_colors()),
            values: Arc::new(values),
            mesh,
            combs,
            curvature: spec.curvature.name().to_string(),
            unit,
            legend: spec.legend,
        })
    }

    /// The combs of the analysis, as marks over the picture.
    fn comb_marks(&self) -> viewport::Marks {
        viewport::Marks {
            combs: self
                .analysis_view
                .as_ref()
                .map(|a| a.combs.clone())
                .unwrap_or_default(),
            ..viewport::Marks::default()
        }
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
        if let Some(first) = scene.problems.first().or(scene.notices.first()) {
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
        // Who made each feature, from the document's provenance (an agent's
        // name; none for the person's own): read again when the document
        // or its revision changes, not every frame.
        let key = (session.document_status().map(|s| s.id), scene.revision);
        if self.provenance.0 != Some(key) {
            self.provenance = (Some(key), provenance(session));
        }
        let made = &self.provenance.1;
        self.rows = scene
            .features
            .iter()
            .map(|f| TreeRow {
                made_by: made.get(&f.id).cloned(),
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
                zoomable: false,
                when: f.when.clone(),
                passes: f.passes.clone(),
                instance: None,
                depth: 0,
            })
            .collect();
        // An assembly lists its instances, nested by subassembly, and its
        // mates (docs/design/assemblies.md › *The tree*).
        self.outline = session.assembly_outline();
        if let Some(status) = session.document_status() {
            if self.outline.is_some() {
                self.assembly_docs.insert(status.id);
            } else {
                self.assembly_docs.remove(&status.id);
            }
            if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.active) {
                tab.assembly = self.outline.is_some();
            }
        }
        if let Some(outline) = &self.outline {
            self.rows = outline
                .instances
                .iter()
                .map(|i| {
                    let id = i.features.first().cloned().unwrap_or_default();
                    TreeRow {
                        selected: !id.is_empty() && selected == Some(id.as_str()),
                        id,
                        name: i.label.clone(),
                        kind: if i.subassembly {
                            "subassembly"
                        } else {
                            "instance"
                        },
                        state: if i.error.is_some() {
                            FeatureState::Failed
                        } else {
                            FeatureState::Ok
                        },
                        status: i.source.clone(),
                        error: i.error.clone(),
                        caused_by: None,
                        hidden: !i.visible,
                        made_selection: false,
                        zoomable: false,
                        when: None,
                        passes: None,
                        instance: i.own.then(|| i.name.clone()),
                        depth: i.depth,
                        made_by: None,
                    }
                })
                .collect();
        }
        self.sync_joint_handles(session, &scene);
        // Framed on a box that does not depend on how finely the scene
        // was sampled: the quick first picture turning into the display
        // scene moves no camera.
        if self.state.camera.auto_fit && scene.frame_box() != self.framed {
            self.framed = scene.frame_box();
            let aspect = self.aspect();
            self.state.camera.fit(&self.framed, aspect);
        }
        Some(scene)
    }

    /// Frames everything, and keeps following the model from now on.
    /// Returns false when there is nothing to frame.
    pub fn fit(&mut self, session: &mut Session) -> bool {
        let Some(scene) = self.sync(session) else {
            return false;
        };
        let frame = scene.frame_box();
        if frame.is_empty() {
            return false;
        }
        let aspect = self.aspect();
        self.state.camera.fit(&frame, aspect);
        self.state.camera.auto_fit = true;
        self.framed = frame;
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
        // Worked out again where the viewport is drawn this frame; a frame
        // without it (the start screen) leaves no stale light look.
        self.interacting = false;
        // Set again where the drawer is drawn.
        self.drawer_sliding = false;
        // The viewport may show a provisional moment of a playing motion,
        // a picture that is recorded or captured may not.
        if let Some(session) = session.as_deref_mut() {
            session.set_motion_live(!self.exact_pictures());
        }
        // The appearance: the panels' colours follow the state's choice,
        // or the system's (`Appearance::System`), from this frame on.
        // The desktop's own setting where the windowing library has none
        // (Linux: the host reads it, `apps/craft3d/src/desktop_theme.rs`).
        self.system_theme = ui.ctx().system_theme().or(self.desktop_theme);
        let dark = self.state.appearance.is_dark(self.system_theme);
        if !self.themed || dark != self.dark {
            self.set_appearance(dark);
            self.tokens.apply(ui.ctx());
            self.themed = true;
        }
        self.fonts.update(ui);
        self.player_shown_last = std::mem::take(&mut self.player_shown);
        self.player_hits.clear();
        // A finger was seen: touch sizes from now on, at any width.
        if ui.input(egui::InputState::any_touches) {
            self.touch_seen = true;
            self.last_touch = ui.input(|i| i.time);
        }
        let width = ui.ctx().content_rect().width();
        self.narrow = width.is_finite() && width < self.tokens.narrow_width;
        let touch_sizes = self.narrow || self.touch_seen;
        if touch_sizes != self.touch_sized {
            self.set_touch_sizes(ui, touch_sizes);
        }
        // The dialogs first: they take Enter and Escape before anything
        // else in the frame reads them (`dialogs.rs`).
        self.dialogs_ui(ui.ctx());
        self.params_frame(ui.ctx());
        let (scene, status) = match session.as_deref_mut() {
            Some(session) => {
                self.length_unit = session.length_unit().unwrap_or("");
                (self.sync(session), session.document_status())
            }
            None => (self.shown_scene.clone(), self.shown_status.clone()),
        };
        self.file_shortcuts(ui, status.is_some());
        self.edit_shortcuts(ui, status.as_ref());
        self.tab_shortcuts(ui);
        self.view_shortcuts(ui);
        self.help_keys(ui);
        // The next step of the history player, when it is time.
        self.advance_player(ui.ctx());
        self.settle_work(ui.ctx());
        let tokens = self.tokens;
        egui::Panel::top("craft3d-menu")
            .resizable(false)
            .show_separator_line(false)
            .frame(chrome_frame(tokens.menu_fill))
            .show(ui, |ui| {
                ui.style_mut().visuals = tokens.chrome_visuals();
                self.menu_bar(ui, status.as_ref());
            });
        if self.file_action.is_some() || self.doc_action.is_some() || !self.edit_actions.is_empty()
        {
            // The host does it between frames: have one come.
            ui.ctx().request_repaint();
        }
        if self.state.panels.recovery && !self.recovery.found.is_empty() {
            let clicked = egui::Panel::top("craft3d-recovery")
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(color(tokens.attention_fill))
                        .stroke(egui::Stroke::new(1.0, color(tokens.attention)))
                        .inner_margin(egui::Margin::symmetric(10, 4)),
                )
                .show(ui, |ui| {
                    ui.style_mut().visuals = tokens.panel_visuals();
                    self.recovery_bar(ui)
                })
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
        } else {
            // No status bar, no pill (its popover opens from the Help menu).
            self.agents.pill = None;
            self.status_hits.clear();
        }
        self.cause_hits.clear();
        // Put away while the welcome shows (`start.rs`).
        let drawer = self.narrow && self.state.panels.tree && !self.welcome();
        if self.state.panels.tree && !self.welcome() && !self.narrow {
            // No margin on the right: the list's scroll bar lies against
            // the panel's edge, the split it is resized by.
            let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin {
                left: side_inset(&tokens),
                right: 0,
                top: 2,
                bottom: 2,
            });
            // The width is the person's: exact, so a long row is cut or
            // wrapped inside it and never widens it; only the split,
            // dragged, changes it (`tree_split`).
            let width = tree_width(
                &tokens,
                self.state.panels.tree_width,
                ui.ctx().content_rect().width(),
            );
            let panel = egui::Panel::left("craft3d-tree")
                .resizable(false)
                .exact_size(width)
                .frame(frame)
                .show(ui, |ui| self.side_panel(ui, status.is_some()));
            self.tree_panel = Some(panel.response.rect);
        } else {
            self.tree_panel = None;
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, scene.as_ref()));
        // The side panel's split, over the viewport's edge: interacted
        // after it, so a drag there resizes the panel and does not orbit.
        if let Some(panel) = self.tree_panel.filter(|_| !self.narrow) {
            self.tree_split(ui, panel);
        }
        // In the narrow layout the side panel is a drawer over the
        // viewport, opened by the button in the menu bar.
        if drawer {
            self.drawer(ui, status.is_some());
        } else if self.narrow {
            self.tree_panel = None;
        }
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
        if self.motion_settling() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(33));
        }
        // What the side panel asked for: edits, each one undo step.
        if let Some(session) = session.as_deref_mut()
            && self.apply_side_requests(session)
        {
            self.sync(session);
            ui.ctx().request_repaint();
        }
        // A tab chosen since the last sync that shows what was not read.
        if let Some(session) = session.as_deref_mut()
            && self.side_stale()
        {
            let scene = self.shown_scene.clone();
            self.sync_side(session, scene.as_deref());
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

    /// The File shortcuts, wherever the pointer is; not under a modal
    /// dialog or the question before closing.
    fn file_shortcuts(&mut self, ui: &egui::Ui, has_document: bool) {
        if self.dialog_open() || self.closing.is_some() {
            return;
        }
        // Shift+S before S: the longer shortcut would also match the
        // shorter one.
        for action in [
            FileAction::SaveAs,
            FileAction::Save,
            FileAction::Open,
            FileAction::NewAssembly,
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

    fn menu_bar(&mut self, ui: &mut egui::Ui, status: Option<&DocumentStatus>) {
        let drawer = self.narrow && self.state.panels.tree && !self.welcome();
        // The menus are the kit's plates, whatever the panels' appearance.
        let menus = self.tokens.menu_visuals();
        let config =
            egui::containers::menu::MenuConfig::new().style(move |style: &mut egui::Style| {
                egui::containers::menu::menu_style(style);
                style.visuals = menus.clone();
                style.spacing.menu_margin = egui::Margin::same(4);
            });
        egui::MenuBar::new().config(config).ui(ui, |ui| {
            // The narrow layout's drawer: the side panel, behind a button.
            self.drawer_button = None;
            if drawer {
                let open = self.state.panels.drawer;
                let side = self.tokens.touch_target;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click());
                let ink = if open || response.hovered() {
                    color(self.tokens.accent)
                } else {
                    ui.visuals().text_color()
                };
                let (w, gap) = (
                    side * self.tokens.drawer_icon_width,
                    side * self.tokens.drawer_icon_gap,
                );
                for k in [-1.0, 0.0, 1.0] {
                    let y = rect.center().y + k * gap;
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.center().x - w * 0.5, y),
                            egui::pos2(rect.center().x + w * 0.5, y),
                        ],
                        egui::Stroke::new(self.tokens.nav_icon_width, ink),
                    );
                }
                let response = response.on_hover_text(if open {
                    "Close the features panel"
                } else {
                    "The features, parameters and player"
                });
                if response.clicked() {
                    self.state.panels.drawer = !open;
                    ui.ctx().request_repaint();
                }
                self.drawer_button = Some(rect);
            }
            self.menus(ui, status);
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
        // Not while an edit made where the bodies are waits: said, so the
        // person clicks again once it is done.
        if session.edits_waiting() {
            self.last_command = Some(CommandReport {
                label: command.to_string(),
                ok: false,
                text: "an edit is still being made; try again once it is done".to_string(),
            });
            return;
        }
        let outcome = session.execute(command, &params);
        let (ok, text) = match &outcome {
            Ok(result) => (true, crate::answer::summary(command, result)),
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
    /// behind (board s6): the attention colour, since it waits for a
    /// decision; Later, Discard all and, for each document, Recover (the
    /// amber action) and Discard. Returns the button that was clicked.
    fn recovery_bar(&mut self, ui: &mut egui::Ui) -> Option<RecoveryAction> {
        use crate::kit::{Button, Glyph, Kit};
        let tokens = self.tokens;
        let kit = Kit::panel(&tokens);
        let dim = color(tokens.text_dim);
        let now = epoch_now();
        let mut hits = Vec::new();
        let mut clicked = None;
        let mut button = |ui: &mut egui::Ui, action: RecoveryAction, b: Button<'_>| {
            let response = kit.button(ui, b);
            if response.clicked() {
                clicked = Some(action.clone());
            }
            hits.push((action, response.rect));
        };
        let armed = |action: &RecoveryAction| self.discard_armed.as_ref() == Some(action);
        let count = self.recovery.found.len();
        ui.add_space(tokens.inset * 0.2);
        ui.horizontal(|ui| {
            let side = tokens.nav_icon_size;
            let (icon, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
            crate::kit::glyphs::paint(
                ui.painter(),
                Glyph::Warning,
                icon,
                color(tokens.attention),
                &tokens,
            );
            let what = if count == 1 {
                "an unsaved document was".to_string()
            } else {
                format!("{count} unsaved documents were")
            };
            ui.label(
                egui::RichText::new(format!(
                    "3DCraft did not close properly: {what} autosaved and can be recovered."
                ))
                .font(kit.medium_font(ui))
                .color(color(tokens.text)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let all = RecoveryAction::DiscardAll;
                let label = if armed(&all) {
                    "Discard all for good?"
                } else {
                    "Discard all"
                };
                button(ui, all, Button::danger(label));
                button(ui, RecoveryAction::Later, Button::quiet("Later"));
            });
        });
        for found in self.recovery.found.iter().take(RECOVERY_ROWS) {
            ui.horizontal(|ui| {
                ui.add_space(tokens.nav_icon_size + ui.spacing().item_spacing.x);
                button(
                    ui,
                    RecoveryAction::Open(found.id.clone()),
                    Button::primary("Recover")
                        .icon(Glyph::Undo)
                        .enabled(found.openable),
                );
                let discard = RecoveryAction::Discard(found.id.clone());
                let label = if armed(&discard) {
                    "Discard for good?"
                } else {
                    "Discard"
                };
                button(ui, discard, Button::quiet(label));
                ui.label(
                    egui::RichText::new(&found.name)
                        .font(kit.medium_font(ui))
                        .color(color(tokens.text)),
                );
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
        ui.add_space(tokens.inset * 0.2);
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
        let font = egui::TextStyle::Body.resolve(ui.style());
        let painter = ui.painter().clone();
        let measure = |text: &str| {
            painter
                .layout_no_wrap(text.to_string(), font.clone(), dim)
                .size()
                .x
        };
        // The texts that change while work or a motion runs change at
        // most once every `status_hold_seconds` (an error at once), and
        // keep their width while only their numbers change
        // (`shell/calm.rs`).
        let now = ui.input(|i| i.time);
        let every = f64::from(tokens.status_hold_seconds);
        let mut again: Option<f64> = None;
        let mut soon = |a: Option<f64>| {
            if let Some(a) = a {
                again = Some(again.map_or(a, |b: f64| b.min(a)));
            }
        };
        match status {
            Some(status) => {
                let units = format!("{} · {}", status.length_unit, status.angle_unit);
                items.push(StatusItem::fixed(
                    StatusSlot::Units,
                    units,
                    color(tokens.chrome_text),
                ));
                // In the narrow layout the tab names the document.
                if !self.narrow {
                    let name = status.path.as_deref().unwrap_or("untitled");
                    let dirty = if status.dirty { " *" } else { "" };
                    items.push(StatusItem::elided(
                        StatusSlot::Path,
                        format!("{name}{dirty}"),
                        color(tokens.chrome_text),
                    ));
                }
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
        // While the motion plays: one steady word, `Playing`, not each
        // moment's answer (its `t` is the player's, which shows it as it
        // runs; 2026-10-30); a moment asked for by `view.motion` says its
        // `t` as the player does.
        let shown_t = self.timeline.motion_t().filter(|t| t.is_finite());
        let line = if self.state.timeline.playing && self.playing_motion() {
            Some(("Playing".to_string(), String::new(), true))
        } else {
            self.last_command.as_ref().map(|report| {
                let answer = match shown_t {
                    Some(_) if report.ok && report.label == "view.motion" => {
                        player::t_text(self.shown_t().or(shown_t).unwrap_or(0.0))
                    }
                    // An answer is said in words where it is made
                    // (`answer::summary`); raw JSON that still gets here
                    // is not shown.
                    _ if report.ok && crate::answer::looks_like_json(&report.text) => String::new(),
                    _ => report.text.clone(),
                };
                (report.label.clone(), answer, report.ok)
            })
        };
        if let Some((label, answer, ok)) = line {
            let joined = format!("{}\u{1f}{label}\u{1f}{answer}", u8::from(ok));
            let (shown, wait) = self.calm.command.frame(now, &joined, !ok, every);
            soon(wait);
            let mut parts = shown.splitn(3, '\u{1f}');
            let ok = parts.next() == Some("1");
            let label = parts.next().unwrap_or_default().to_string();
            let answer = parts.next().unwrap_or_default().to_string();
            let tint = if ok {
                tokens.chrome_ok
            } else {
                tokens.chrome_failed
            };
            let label_width = self
                .calm
                .command_width
                .width(&label, measure(&label).ceil());
            let answer_width = self
                .calm
                .answer_width
                .width(&answer, measure(&answer).ceil());
            items.push(StatusItem {
                width: Some(label_width),
                ..StatusItem::fixed(StatusSlot::Command, label, color(tint))
            });
            items.push(StatusItem {
                width: Some(answer_width),
                ..StatusItem::elided(StatusSlot::Answer, answer, dim)
            });
        }
        if let Some(text) = self.calm.work_shown.clone() {
            items.push(StatusItem {
                width: Some(tokens.status_work_width),
                ..StatusItem::elided(StatusSlot::Work, text, color(tokens.chrome_text))
            });
        }
        if let Some(stats) = self.stats {
            // How long the picture took is the wall clock's: not drawn
            // while a frame-exact recording keeps the clock.
            let time = stats
                .milliseconds
                .filter(|_| !self.frame_exact)
                .map_or_else(String::new, |ms| format!(" · {ms:.0} ms"));
            // The quick first picture is shown while the display mesh is
            // built: said here, unobtrusively.
            let refining = if self.shown_scene.as_ref().is_some_and(|s| s.preview) {
                "refining · "
            } else {
                ""
            };
            // In the narrow layout only what a person acts on: that the
            // sharp picture is still coming.
            let text = if self.narrow {
                refining.trim_end_matches(" · ").to_string()
            } else {
                format!(
                    "{refining}{} {}×{} · {} triangles{time}",
                    stats.renderer.name().to_uppercase(),
                    stats.width,
                    stats.height,
                    stats.triangles
                )
            };
            let (text, wait) = self.calm.render.frame(now, &text, false, every);
            let text = text.to_string();
            soon(wait);
            let width = self.calm.render_width.width(&text, measure(&text).ceil());
            items.push(StatusItem {
                width: Some(width),
                ..StatusItem::fixed(StatusSlot::Render, text, dim)
            });
        }
        if let Some(again) = again {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(again.clamp(0.0, 1.0)));
        }
        let texts: Vec<(StatusSlot, String)> = items
            .iter()
            .filter(|i| {
                matches!(
                    i.slot,
                    StatusSlot::Command
                        | StatusSlot::Answer
                        | StatusSlot::Work
                        | StatusSlot::Render
                )
            })
            .map(|i| (i.slot, i.text.clone()))
            .collect();
        if texts != self.calm.last {
            self.calm.changes = self.calm.changes.saturating_add(1);
            self.calm.last = texts;
        }
        let fits: Vec<StatusFit> = items
            .iter()
            .map(|item| {
                let natural = item.width.unwrap_or_else(|| measure(&item.text).ceil());
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
        // The agents pill at the right end (decision of 2026-11-02,
        // `shell/agents.rs`; none in the browser): its room, at most two
        // fifths of the bar, is kept out of what the items get.
        let pill_most = (ui.available_width() * 0.4).max(80.0);
        let pill_width = self.agents_pill_width(ui, pill_most);
        let pill_room = if pill_width > 0.0 {
            pill_width + tokens.status_gap
        } else {
            0.0
        };
        let available = (ui.available_width() - pill_room).max(0.0);
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
        // The items on the right, with the gaps between them: the room
        // left goes before the first of them.
        let right: Vec<f32> = items
            .iter()
            .zip(&widths)
            .filter(|(item, _)| item.on_the_right())
            .filter_map(|(_, w)| *w)
            .collect();
        let right_total =
            right.iter().sum::<f32>() + tokens.status_gap * right.len().saturating_sub(1) as f32;
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
            let mut right_started = false;
            for (item, width) in items.iter().zip(&widths) {
                let Some(width) = *width else { continue };
                if item.on_the_right() && !copy_shown {
                    copy_button(ui, &mut used);
                    copy_shown = true;
                }
                if item.on_the_right() && !right_started {
                    // Right-aligned: the room left goes before them.
                    right_started = true;
                    let room = (available - used - right_total).max(0.0);
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
                // The slot is exactly `width` wide, whatever its words
                // measure in the font in force: a truncated text a fraction
                // wider than its room would otherwise widen the slot and
                // move the items after it.
                let (slot, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                let mut inside = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(slot)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                inside.set_clip_rect(slot.intersect(ui.clip_rect()));
                if item.slot == StatusSlot::Work {
                    // The work indicator: the kit's spinner (on the
                    // chrome, the plate's inks) before the words.
                    crate::kit::Kit::plate(&tokens).spinner(&mut inside, "");
                }
                let response = inside.add(label);
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
            if pill_width > 0.0 {
                ui.add_space((available - used).max(0.0) + tokens.status_gap);
                self.agents_pill(ui, pill_most);
            } else {
                self.agents.pill = None;
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

    /// The panels' colours of the dark or the light appearance, with the
    /// sizes kept (touch sizes too, and the desktop's kept aside for when
    /// they end).
    fn set_appearance(&mut self, dark: bool) {
        self.dark = dark;
        let palette = if dark {
            crate::theme::Palette::dark()
        } else {
            crate::theme::Palette::light()
        };
        self.tokens = self.tokens.with_palette(palette);
        if let Some((tokens, _)) = self.wide.as_mut() {
            *tokens = tokens.with_palette(palette);
        }
    }

    /// The desktop's light or dark setting, as the host read it where the
    /// windowing library reports none (GNOME on Linux); `None` for no
    /// preference. Appearance › System follows it.
    pub fn set_desktop_theme(&mut self, theme: Option<egui::Theme>) {
        self.desktop_theme = theme;
    }

    /// Whether the panels have the dark appearance now (as drawn by the
    /// last frame).
    pub fn dark(&self) -> bool {
        self.dark
    }

    /// Whether the panels are dark by the state as it is now, against the
    /// system's preference the last frame saw: what the next frame draws.
    pub fn resolved_dark(&self) -> bool {
        self.state.appearance.is_dark(self.system_theme)
    }

    /// Into touch sizes or out of them (the narrow layout, or a finger seen
    /// on the screen): the controls and egui's own widgets as large as a
    /// finger needs, or as they were.
    fn set_touch_sizes(&mut self, ui: &mut egui::Ui, touch: bool) {
        self.touch_sized = touch;
        if touch {
            let spacing = ui.ctx().global_style().spacing.clone();
            self.wide = Some((self.tokens, spacing));
            self.tokens = self.tokens.touch();
            let target = self.tokens.touch_target;
            ui.ctx().all_styles_mut(move |style| {
                style.spacing.interact_size.y = style.spacing.interact_size.y.max(target);
                style.spacing.button_padding.y = style.spacing.button_padding.y.max(target * 0.2);
            });
            *ui.spacing_mut() = ui.ctx().global_style().spacing.clone();
        } else if let Some((tokens, spacing)) = self.wide.take() {
            self.tokens = tokens;
            let restored = spacing.clone();
            ui.ctx()
                .all_styles_mut(move |style| style.spacing = restored.clone());
            *ui.spacing_mut() = spacing;
        }
    }

    /// Whether the shell has the narrow layout now (a window narrower
    /// than `Tokens::narrow_width`).
    pub fn narrow(&self) -> bool {
        self.narrow
    }

    /// Where the drawer's button is in the menu bar, in the narrow layout.
    pub fn drawer_button_rect(&self) -> Option<egui::Rect> {
        self.drawer_button
    }

    /// The side panel as a drawer over the viewport, from its left edge,
    /// as wide as the panel (at most `drawer_share` of the window).
    fn drawer(&mut self, ui: &mut egui::Ui, has_document: bool) {
        let tokens = self.tokens;
        let Some(viewport) = self.rect else {
            self.tree_panel = None;
            return;
        };
        let window = ui.ctx().content_rect().width();
        let width = (self
            .state
            .panels
            .tree_width
            .clamp(tokens.tree_min_width, tokens.tree_max_width))
        .min(window * tokens.drawer_share)
        .min(viewport.width());
        if !(width.is_finite() && width > 0.0) {
            self.tree_panel = None;
            return;
        }
        // It slides in from the left and out again, in a tenth of a second.
        let shown = ui.ctx().animate_bool_with_time(
            egui::Id::new("craft3d-drawer-open"),
            self.state.panels.drawer,
            tokens.drawer_seconds,
        );
        let target = if self.state.panels.drawer { 1.0 } else { 0.0 };
        self.drawer_sliding = shown.is_finite() && (shown - target).abs() > 1e-3;
        if shown <= 0.0 || !shown.is_finite() {
            self.tree_panel = None;
            return;
        }
        let rect = egui::Rect::from_min_size(
            viewport.min - egui::vec2((1.0 - shown) * width, 0.0),
            egui::vec2(width, viewport.height()),
        );
        let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin {
            left: side_inset(&tokens),
            right: 0,
            top: 2,
            bottom: 2,
        });
        egui::Area::new(egui::Id::new("craft3d-drawer"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ui.ctx(), |ui| {
                ui.set_clip_rect(rect.intersect(viewport));
                let inner = rect - frame.total_margin();
                frame.show(ui, |ui| {
                    ui.set_min_size(inner.size());
                    ui.set_max_size(inner.size());
                    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                        self.side_panel(ui, has_document);
                    });
                });
                // A line along its open edge.
                ui.painter().line_segment(
                    [rect.right_top(), rect.right_bottom()],
                    egui::Stroke::new(tokens.divider_width, color(tokens.plate_rim)),
                );
            });
        // Its place while it is open; closing, it is on its way out.
        self.tree_panel = self.state.panels.drawer.then(|| rect.intersect(viewport));
    }

    /// The split on the right of the side panel `panel`: dragged, it sets
    /// the panel's width (`state.panels.treeWidth`) within the tokens'
    /// range. Interacted
    /// after the panel's contents, so it lies over its scroll bar.
    fn tree_split(&mut self, ui: &mut egui::Ui, panel: egui::Rect) {
        let tokens = self.tokens;
        let grip = egui::Rect::from_x_y_ranges(
            panel.right() - tokens.tree_split_grip..=panel.right(),
            panel.y_range(),
        );
        // Inside the panel's edge, so every point of the viewport stays
        // the viewport's.
        let split = ui.interact(
            grip,
            egui::Id::new("craft3d-tree-split"),
            egui::Sense::drag(),
        );
        if split.hovered() || split.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            ui.painter().line_segment(
                [
                    egui::pos2(panel.right(), panel.top()),
                    egui::pos2(panel.right(), panel.bottom()),
                ],
                egui::Stroke::new(tokens.divider_width * 2.0, color(tokens.accent)),
            );
        }
        let window = ui.ctx().content_rect().width();
        if split.dragged()
            && let Some(at) = split.interact_pointer_pos()
        {
            self.state.panels.tree_width = tree_width(&tokens, at.x - panel.left(), window);
            ui.ctx().request_repaint();
        }
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
            let hint = if self.outline.is_some() {
                "No instances yet."
            } else if has_document {
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
        let mut instance_eye = None;
        let mut open_instance = None;
        let mut instance_drawn = Vec::new();
        let in_use = self.jig_in_use();
        let outline = self.outline.clone();
        let mut hits = Vec::new();
        let scroll_to = self.tree_scroll.take();
        let mut rows_drawn = Vec::new();
        let mut zooms_drawn = Vec::new();
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
                                FeatureState::Suppressed
                                | FeatureState::Inactive
                                | FeatureState::RolledBack => tokens.inactive,
                            };
                            let mark = tree_mark(row.state);
                            let mut name = egui::RichText::new(&row.name);
                            match row.state {
                                FeatureState::Ok => {}
                                FeatureState::Failed => name = name.color(color(tokens.failed)),
                                FeatureState::Suppressed => {
                                    name = name.color(color(tokens.inactive)).strikethrough();
                                }
                                // Off by its condition: greyed, not struck
                                // through (nobody switched it off by hand).
                                FeatureState::Inactive => {
                                    name = name.color(color(tokens.inactive));
                                }
                                FeatureState::RolledBack => {
                                    name = name.color(color(tokens.inactive)).italics()
                                }
                            }
                            // What it passes on, from the regeneration: a
                            // body, or none and why, and what it left out.
                            let passes = row.passes.as_deref().unwrap_or("");
                            let hover = match (&row.error, row.state) {
                                (Some(error), _) => error.clone(),
                                (None, FeatureState::Suppressed) => {
                                    format!("Suppressed: not built. {passes}")
                                        .trim_end()
                                        .to_string()
                                }
                                (None, FeatureState::Inactive) => format!(
                                    "Inactive: its condition {} is 0; not built. {passes}",
                                    row.when.as_deref().unwrap_or("")
                                )
                                .trim_end()
                                .to_string(),
                                (None, FeatureState::Ok) if !passes.is_empty() => {
                                    format!("{} · {} · {passes}", row.kind, row.status)
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
                                // Nested in its subassembly.
                                if row.depth > 0 {
                                    ui.add_space(row.depth.min(8) as f32 * tokens.inset);
                                }
                                ui.colored_label(color(tint), mark).on_hover_text(&hover);
                                // The eye, Open and the zoom icon against the
                                // right edge, laid out first so a long name
                                // never pushes them out of the panel.
                                let row_hovered = ui.rect_contains_pointer(ui.max_rect());
                                let trailing = egui::Layout::right_to_left(egui::Align::Center);
                                ui.with_layout(trailing, |ui| {
                                    // A Jig in use is looked at, not changed.
                                    if outline.is_some() {
                                        // An assembly's own instance: shown or
                                        // hidden, and opened in a tab of its own.
                                        if let Some(instance) = &row.instance {
                                            let open = ui.small_button("Open").on_hover_text(
                                                "Open its source in a tab of its own",
                                            );
                                            instance_drawn.push((
                                                instance.clone(),
                                                "open",
                                                open.rect,
                                            ));
                                            if open.clicked() {
                                                open_instance = Some(instance.clone());
                                            }
                                            let eye = timeline::eye(ui, row.hidden, &tokens);
                                            instance_drawn.push((
                                                instance.clone(),
                                                "eye",
                                                eye.rect,
                                            ));
                                            if eye.clicked() {
                                                instance_eye = Some((instance.clone(), row.hidden));
                                            }
                                        }
                                    } else if !in_use
                                        && timeline::eye(ui, row.hidden, &tokens).clicked()
                                    {
                                        eye_clicked = Some((row.id.clone(), row.hidden));
                                    }
                                    if let Some(icon) =
                                        views::zoom_icon(ui, row.zoomable, row_hovered, &tokens)
                                    {
                                        zooms_drawn.push((row.id.clone(), icon.rect));
                                        if icon.clicked() {
                                            double = Some(row.id.clone());
                                        }
                                    }
                                    let leading = egui::Layout::left_to_right(egui::Align::Center);
                                    ui.with_layout(leading, |ui| {
                                        // A long name is cut to the room left
                                        // (the whole name on hover), never
                                        // widening the panel.
                                        let label = ui
                                            .add(
                                                egui::Button::selectable(
                                                    row.selected || row.made_selection,
                                                    name,
                                                )
                                                .truncate(),
                                            )
                                            .on_hover_text(&hover);
                                        // A row of an instance inside a rigid
                                        // subassembly has no body of its own.
                                        let selectable = !row.id.is_empty();
                                        if selectable {
                                            rows_drawn.push((row.id.clone(), label.rect));
                                        }
                                        // The second click of a double click is a
                                        // click too: the double click wins, and
                                        // does nothing on a row that cannot be
                                        // flown to.
                                        if !selectable {
                                        } else if label.double_clicked() {
                                            if row.zoomable {
                                                double = Some(row.id.clone());
                                            }
                                        } else if label.clicked() {
                                            clicked = Some(row.id.clone());
                                        }
                                        if scroll_to.as_deref() == Some(row.id.as_str()) {
                                            // Only as far as it takes (a row in
                                            // view stays), and at once: the list
                                            // follows a click in the viewport, it
                                            // does not glide.
                                            label.scroll_to_me_animation(
                                                None,
                                                egui::style::ScrollAnimation::none(),
                                            );
                                        }
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(format!(
                                                    "{} · {}",
                                                    row.kind, row.status
                                                ))
                                                .color(dim),
                                            )
                                            .truncate(),
                                        );
                                        // A condition: a small mark, the
                                        // expression on hover.
                                        if let Some(when) = &row.when {
                                            ui.colored_label(
                                                dim,
                                                egui::RichText::new("when")
                                                    .size(tokens.secondary_font),
                                            )
                                            .on_hover_text(format!("Built when {when} is not 0"));
                                        }
                                    });
                                });
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
                                                .size(tokens.secondary_font),
                                        )
                                        .wrap(),
                                    );
                                });
                            }
                        }
                        if let Some(outline) = &outline {
                            mates_ui(ui, &tokens, outline);
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
                                // Switched off by its condition, as a
                                // feature by its when.
                                A::Inactive => (FeatureState::Inactive, tokens.inactive),
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
                                ui.add(egui::Label::new(name).truncate())
                                    .on_hover_text(&row.message);
                                ui.add(
                                    egui::Label::new(egui::RichText::new(status).color(dim))
                                        .truncate(),
                                );
                            });
                            if matches!(row.status, A::Failed | A::Error | A::NotProven) {
                                ui.indent(("craft3d-assertion", &row.name), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&row.message)
                                                .color(color(tint))
                                                .size(tokens.secondary_font),
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
        self.instance_hits = instance_drawn;
        if let Some((name, hidden)) = instance_eye {
            self.timeline.queue(
                "instance.edit",
                serde_json::json!({"name": name, "visible": hidden}),
            );
            ui.ctx().request_repaint();
        }
        if let Some(name) = open_instance {
            self.timeline
                .queue("instance.open", serde_json::json!({"name": name}));
            ui.ctx().request_repaint();
        }
        if let Some((id, hidden)) = eye_clicked {
            self.toggle_visible(id, hidden);
            ui.ctx().request_repaint();
        }
        self.tree_row_hits = rows_drawn;
        self.tree_zoom_hits = zooms_drawn;
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

    /// What is being done, for the work indicator in the status bar
    /// ([`Self::work_shown`]): the words, and how far along it is (0 to 1)
    /// when that is known. The host's work (opening, regenerating, meshing), else the
    /// assertions still being measured. `None` when nothing is.
    pub fn busy(&self) -> Option<(String, Option<f32>)> {
        if let Some(work) = &self.work {
            return Some((work.status_text(), work.fraction()));
        }
        let of = self.assertion_rows.len();
        let pending = self
            .assertion_rows
            .iter()
            .filter(|a| a.status == craft3d_engine::AssertionStatus::Pending)
            .count();
        if pending == 0 {
            return None;
        }
        let done = of.saturating_sub(pending);
        let noun = if of == 1 { "assertion" } else { "assertions" };
        let text = if done == 0 {
            format!("Checking {of} {noun}\u{2026}")
        } else {
            format!("Checking {of} {noun}\u{2026} {done}/{of}")
        };
        let fraction = (done > 0).then(|| (done as f32 / of.max(1) as f32).clamp(0.0, 1.0));
        Some((text, fraction))
    }

    /// Decides for this frame whether the work indicator shows and what
    /// it says: only work that has run `Tokens::work_after_seconds`, and
    /// then at least `Tokens::work_hold_seconds` (`shell/calm.rs`). Short
    /// work, like each moment of a motion played, never shows it.
    fn settle_work(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let tokens = self.tokens;
        let busy = self.busy().map(|(text, _)| text);
        let shown = self.calm.work.frame(
            now,
            busy.as_deref(),
            f64::from(tokens.work_after_seconds),
            f64::from(tokens.work_hold_seconds),
            f64::from(tokens.status_hold_seconds),
        );
        if let Some(again) = shown.again_in {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(again.clamp(0.0, 1.0)));
        }
        if shown.text.is_some() {
            // The spinner turns.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.calm.work_shown = shown.text;
    }

    /// What the work indicator says in the status bar, when it shows
    /// (`ui.get` › `workShown`). [`Self::busy`] says what is being done
    /// whether it shows or not.
    pub fn work_shown(&self) -> Option<&str> {
        self.calm.work_shown.as_deref()
    }

    /// How many times the texts of the status bar have changed from one
    /// frame to the next since the window opened: what a test counts to
    /// see that they do not flicker.
    pub fn status_changes(&self) -> u64 {
        self.calm.changes
    }

    /// What the status bar showed in the last frame for the last command
    /// (or `Playing`), its answer, the work and the renderer's numbers.
    pub fn status_shown(&self, slot: StatusSlot) -> Option<&str> {
        self.calm
            .last
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, text)| text.as_str())
    }

    /// While the viewport has nothing to show yet (a first file opening,
    /// the page before its first picture): a small note in its middle,
    /// as wide whatever it says, with what is opening.
    fn work_note(&self, painter: &egui::Painter, rect: egui::Rect, work: &Work) {
        let tokens = self.tokens;
        let text = match (work.label.as_str(), work.subject.as_deref()) {
            ("file.open", Some(name)) => format!("Opening {name}\u{2026}"),
            ("file.open", None) => "Opening\u{2026}".to_string(),
            _ => "Working\u{2026}".to_string(),
        };
        let width = tokens
            .work_note_width
            .min(rect.width() - 2.0 * tokens.inset);
        // Room for words inside the plate's padding; none in a viewport
        // too small for that (or with sizes that are not numbers).
        let room = width - 2.0 * tokens.inset;
        if !(room.is_finite() && room > 0.0) {
            return;
        }
        let font = egui::FontId::proportional(15.0);
        let job = egui::text::LayoutJob {
            wrap: egui::text::TextWrapping::truncate_at_width(room),
            ..egui::text::LayoutJob::simple_singleline(text, font, color(tokens.notice_text))
        };
        let galley = painter.layout_job(job);
        let height = galley.size().y + tokens.inset;
        let plate = egui::Rect::from_center_size(rect.center(), egui::vec2(width, height));
        painter.rect_filled(
            plate,
            egui::CornerRadius::same(tokens.notice_radius),
            color_alpha(tokens.notice_fill, tokens.notice_opacity),
        );
        painter.galley(
            plate.center() - galley.size() * 0.5,
            galley,
            color(tokens.notice_text),
        );
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
        // (On the direct path there is no texture of the CPU's; a picture
        // it showed counts the same.)
        let resized = size != self.size && (self.texture.is_some() || self.direct_shown);
        if size != self.size {
            self.size = size;
            // The model was framed for another shape of viewport.
            self.framed = Bbox3::EMPTY;
            if self.state.camera.auto_fit
                && let Some(scene) = scene
            {
                self.framed = scene.frame_box();
                let aspect = self.aspect();
                self.state.camera.fit(&self.framed, aspect);
            }
        }

        self.start_requested_fly(scene, ui.input(|i| i.time));
        let turning = self.advance_turn(ui, scene);
        let moving =
            self.pointer(ui, &response, rect, scene) || resized || turning || self.orbiting;
        if let Some(left) = self.note_interaction(moving, ui.input(|i| i.time)) {
            // Back to the full look once the camera has rested.
            ui.ctx().request_repaint_after(
                std::time::Duration::try_from_secs_f64(left).unwrap_or_default(),
            );
        }
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
        // In the narrow layout a tap beside the open drawer closes it and
        // selects nothing.
        // A release that ends a pinch or a two-finger pan is no tap.
        let tapped = response.clicked() && !self.touch_gesture;
        let closes_drawer = self.narrow && self.state.panels.drawer && tapped;
        if closes_drawer {
            self.state.panels.drawer = false;
            ui.ctx().request_repaint();
        }
        if tapped
            && !closes_drawer
            && let Some(at) = response.interact_pointer_pos()
        {
            let (x, y) = pixel(at);
            let keys = ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
            let double = response.double_clicked();
            // A double tap does what F does: flies to what the first tap
            // selected, or frames everything when it selected nothing.
            // Known by its own time and place (a touch in the last second:
            // the release frame of a tap has no touch any more).
            let now = ui.input(|i| i.time);
            let touch = now - self.last_touch < 1.0;
            let double_tap = touch
                && self.last_tap.is_some_and(|(then, there)| {
                    now - then < f64::from(tokens.double_tap_seconds)
                        && there.distance(at) < tokens.double_tap_distance
                });
            self.last_tap = (touch && !double_tap).then_some((now, at));
            if double_tap {
                self.fit_key(scene, ui.input(|i| i.time));
            // A picture that cannot be made has nothing to point at; the
            // viewport says why already.
            } else if let Ok(found) = self.pick(scene, x, y, None, None, true) {
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
        // Esc closes an open popover of the Studio or Analysis chip first,
        // wherever the pointer is; then it clears the selection over the
        // viewport.
        if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(help::CLEAR_KEY)) {
            if self.close_popover() {
                ui.ctx().request_repaint();
            } else if response.contains_pointer() {
                self.escape_selection();
                ui.ctx().request_repaint();
            }
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
            edges: self.edges_drawn(),
            traces: self.traces_key(),
            look: self.state.look.clone(),
            analysis: self.analysis_serial,
            look_ready: self.look_ready(),
            light: self.playing_light() || self.interacting_light(),
            tokens,
        };
        // The environment is being prepared on a thread, which cannot
        // wake the window: look again soon. (In the browser the page asks
        // for a repaint when it has prepared it.)
        #[cfg(not(target_arch = "wasm32"))]
        if !key.look_ready {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        // The direct path draws the whole picture, lines included, on the
        // window's device, every frame in full; the other paths make a
        // preview here and the sharp image elsewhere.
        self.direct_shown = self.direct_frame(scene, &key);
        if !self.direct_shown {
            self.refresh(ui.ctx(), shared, &key, moving);
        } else if self.direct.as_ref().is_some_and(Direct::converging) {
            // The studio look accumulates frames while nothing moves.
            ui.ctx().request_repaint();
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
                        color(tokens.datum_ink),
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
                color(tokens.viewport_text),
            );
        };
        let start = self.welcome();
        // Nothing drawn yet while a file opens: what is opening, in the
        // middle, gone with the first body.
        let nothing_yet = scene.is_none_or(|scene| scene.bodies.is_empty());
        match (&self.work, scene, self.problem()) {
            (Some(work), _, None) if nothing_yet && self.calm.work_shown.is_some() => {
                self.work_note(&painter, rect, work);
            }
            (None, Some(scene), _) if scene.bbox.is_empty() => hint("Empty document."),
            _ => {}
        }
        // The caption of a narrated demo, then the history player's step
        // under it: plates at the top, in the middle, or in the middle of
        // the room left of the camera plate when they would reach it.
        let mut top = inner.top();
        let captions = [
            (!self.state.notice.is_empty())
                .then(|| (self.state.notice.clone(), tokens.notice_font)),
            // A motion's `t` is the player's while the player is in view.
            self.player_caption()
                .filter(|_| !(self.timeline.motion.is_some() && self.player_shown_last))
                .map(|text| (text, tokens.caption_font)),
        ];
        let plate = self.controls_layout.and_then(|l| l.camera_plate);
        for (text, size) in captions.into_iter().flatten() {
            let galley = painter.layout_no_wrap(
                text,
                egui::FontId::proportional(size),
                color(tokens.notice_text),
            );
            let text_size = galley.size();
            let half = text_size.x * 0.5 + inset * 0.8;
            let x = caption_x(
                rect,
                plate.filter(|p| p.top() < top + 2.0 * text_size.y + inset),
                half,
                tokens.nav_margin,
                tokens.nav_gap,
            );
            let center = egui::pos2(x, top + text_size.y);
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
            .layout_no_wrap(
                problem.to_string(),
                font.clone(),
                color(tokens.viewport_failed),
            )
            .size()
            .x;
        let Some([left, width, bottom]) = layout::problem_place(rect, &layout, &tokens, natural)
        else {
            return;
        };
        let galley = painter.layout(
            problem.to_string(),
            font,
            color(tokens.viewport_failed),
            width,
        );
        let top = bottom - galley.size().y;
        painter.galley(egui::pos2(left, top), galley, color(tokens.viewport_failed));
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
                self.framed = scene.frame_box();
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

    /// Where the side panel on the left (feature tree, Parameters, player)
    /// was drawn, in points, while it is shown.
    pub fn tree_panel_rect(&self) -> Option<egui::Rect> {
        self.tree_panel
    }

    fn fit_to(&mut self, scene: Option<&Scene>) {
        if let Some(scene) = scene
            && !scene.frame_box().is_empty()
        {
            let aspect = self.aspect();
            self.framed = scene.frame_box();
            self.state.camera.fit(&self.framed, aspect);
            self.state.camera.auto_fit = true;
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
        // Two fingers: they pan the view as they move together and zoom
        // about their middle as they pinch; the finger left after them
        // turns nothing until it is lifted.
        if let Some(touch) = ui
            .input(egui::InputState::multi_touch)
            .filter(|t| t.num_touches >= 2)
        {
            self.touch_gesture = true;
            self.drag_from = None;
            if response.contains_pointer() || rect.contains(touch.center_pos) {
                let (dx, dy) = (
                    f64::from(touch.translation_delta.x),
                    f64::from(touch.translation_delta.y),
                );
                if dx.is_finite() && dy.is_finite() && (dx != 0.0 || dy != 0.0) {
                    self.state.camera.pan(dx, dy, f64::from(rect.height()));
                }
                let zoom = f64::from(touch.zoom_delta);
                if zoom > 0.0 && zoom.is_finite() && zoom != 1.0 {
                    let steps = zoom.pln() / crate::camera::ZOOM_PER_POINT;
                    match self.zoom_point(touch.center_pos, rect, scene) {
                        Some(point) => self.state.camera.zoom_about(steps, point),
                        None => self.state.camera.zoom(steps),
                    }
                }
            }
            return true;
        }
        // The gesture lasts until the next press: the release that ends it
        // is not a tap, and the finger left after it turns nothing.
        if self.touch_gesture {
            if ui.input(|i| i.pointer.any_pressed()) {
                self.touch_gesture = false;
            } else if ui.input(|i| i.any_touches() || i.pointer.any_down()) {
                self.drag_from = None;
                return false;
            }
        }
        // Alt and a drag on a part of an assembly move it along its free
        // motion, not the camera.
        if self.part_drag(ui, response, rect, scene) {
            self.drag_from = None;
            return true;
        }
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
                pinch.pln() / crate::camera::ZOOM_PER_POINT
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
        let studio = self.studio();
        let marks = self.frame_marks(scene, key);
        // Smaller while the camera moves, where the host asked for it.
        let divisor = if self.interacting_light() && studio.is_some() {
            self.moving_divisor.max(1)
        } else {
            1
        };
        let (size, _) = moving_size(key.size, self.pixels_per_point, divisor);
        let Some(direct) = &mut self.direct else {
            return false;
        };
        direct.set_highlight(key.highlight);
        direct.set_opacity(key.opacity);
        direct.set_studio(studio);
        direct.set_analysis(self.analysis_view.clone());
        direct.set_marks(marks);
        // The lines stay uploaded at the viewport's pixels per point and
        // are drawn thinner in the smaller picture.
        direct.set_ink(1.0 / divisor as f32);
        let outcome = direct.frame(
            scene,
            &key.camera,
            size,
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
        let marks = self.frame_marks(scene.map(Arc::as_ref), key);
        let studio = self.studio_once();
        let analysis = self.analysis_view.clone();
        let job = |quality: Quality| RenderJob {
            scene: scene.cloned(),
            camera: key.camera,
            quality,
            tokens: key.tokens,
            selected: key.selected.clone(),
            highlight: key.highlight,
            marks: marks.clone(),
            opacity: key.opacity,
            studio: studio.clone(),
            analysis: analysis.clone(),
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

/// The size the direct path draws a viewport of `size` at, `divisor`
/// times smaller per side (at least one pixel), and the pixels per point
/// of that smaller picture. The lines stay uploaded at the viewport's
/// pixels per point and are drawn `1 / divisor` as wide
/// (`Direct::set_ink`), so stretched over the viewport they look as wide
/// as at its full size.
pub fn moving_size(size: [u32; 2], pixels_per_point: f32, divisor: u32) -> ([u32; 2], f32) {
    if divisor <= 1 {
        return (size, pixels_per_point);
    }
    let side = |pixels: u32| pixels.div_ceil(divisor).max(1);
    let [width, height] = size;
    (
        [side(width), side(height)],
        pixels_per_point / divisor as f32,
    )
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
/// How the bodies' edges are drawn: shown or not (`show.edges`), and in
/// the studio look as its `edges` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EdgesDrawn {
    shown: bool,
    mode: crate::state::EdgeMode,
}

/// The marks of a highlighted edge and a highlighted vertex, the section
/// the picture is cut open with, and the datums when they are shown.
fn marks_of(
    scene: Option<&Scene>,
    edge: Option<&EdgeRef>,
    vertex: Option<&VertexRef>,
    cut: Option<viewport::SectionCut>,
    datums: bool,
    sketches: bool,
    edges_drawn: EdgesDrawn,
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
        hide_edges: !edges_drawn.shown || edges_drawn.mode == crate::state::EdgeMode::Off,
        soft_edges: edges_drawn.mode == crate::state::EdgeMode::Soft,
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

/// An assembly's mates under its instances: each with its mark, kind and
/// value (or why it fails), then the degrees of freedom left.
fn mates_ui(ui: &mut egui::Ui, tokens: &Tokens, outline: &craft3d_engine::AssemblyOutline) {
    let dim = color(tokens.text_dim);
    ui.add_space(tokens.inset * 0.5);
    ui.label(egui::RichText::new("Mates").strong());
    ui.separator();
    if outline.mates.is_empty() {
        ui.colored_label(
            dim,
            egui::RichText::new("No mates yet.").size(tokens.secondary_font),
        );
    }
    for mate in &outline.mates {
        let (state, tint) = match mate.status {
            "ok" => (FeatureState::Ok, tokens.ok),
            "unsolved" => (FeatureState::Failed, tokens.attention),
            _ => (FeatureState::Failed, tokens.failed),
        };
        let value = match mate.value {
            Some((v, true)) => format!(" · {:.1}°", v.to_degrees()),
            Some((v, false)) => format!(" · {v:.3}"),
            None => String::new(),
        };
        let driven = if mate.driven { " · driven" } else { "" };
        let hover = mate
            .error
            .clone()
            .unwrap_or_else(|| format!("{} · {}", mate.kind, mate.status));
        ui.horizontal(|ui| {
            ui.colored_label(color(tint), tree_mark(state))
                .on_hover_text(&hover);
            // Cut to the panel's width, never widening it; the whole
            // text on hover.
            ui.add(egui::Label::new(&mate.name).truncate())
                .on_hover_text(&hover);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{}{value}{driven}", mate.kind)).color(dim),
                )
                .truncate(),
            );
        });
        if let Some(error) = &mate.error {
            ui.indent(("craft3d-mate-error", &mate.name), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(error)
                            .color(color(tint))
                            .size(tokens.secondary_font),
                    )
                    .wrap(),
                );
            });
        }
    }
    let left = match (&outline.solve_error, outline.dof) {
        (Some(why), _) => {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Not solved: {why}"))
                        .color(color(tokens.failed))
                        .size(tokens.secondary_font),
                )
                .wrap(),
            );
            return;
        }
        (None, Some(0)) => "Fully constrained (0 degrees of freedom left)".to_string(),
        (None, Some(1)) => "1 degree of freedom left".to_string(),
        (None, Some(n)) => format!("{n} degrees of freedom left"),
        (None, None) => return,
    };
    ui.colored_label(dim, left)
        .on_hover_text(outline.free.join("\n"));
}

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
                    .size(tokens.secondary_font),
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
        let length = (line.to[0] - line.from[0]).phypot(line.to[1] - line.from[1]);
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
        let wide = f64::from(line.width(tokens)) * pixels_per_point;
        for k in 0..pieces {
            let (t0, t1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
            let middle = at(0.5 * (t0 + t1));
            let mut fade = viewport::edge_fade(long, density.at(middle[0], middle[1]), tokens);
            // Where the studio look's lens blurs it, as at the piece's middle.
            if let Some(blur) = &overlay.blur {
                fade *= blur.fade_along(edge.depth, 0.5 * (t0 + t1), wide);
            }
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
