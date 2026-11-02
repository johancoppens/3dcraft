//! The shell's own state: the camera, which panels are open, what is
//! selected and the caption. Plain serde data, so the control channel reads
//! it (`ui.get`) and drives it (`ui.set`) exactly as the mouse does
//! (AGENTS.md rule 9). Nothing here belongs to a document.

use craft3d_engine::{
    DISPLAY_MAX_ANGLE_DEGREES, DISPLAY_MAX_TRIANGLES, DISPLAY_RELATIVE_TOLERANCE, SceneQuality,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Largest coordinate of the camera target, and largest distance, in model
/// units. Larger values are clamped: beyond this `f64` has no precision left
/// for a picture anyway.
pub const MAX_EXTENT: f64 = 1e12;
/// Smallest distance from the eye to the target.
pub const MIN_DISTANCE: f64 = 1e-9;
/// Vertical field of view, in degrees.
pub const MIN_FOV: f64 = 1.0;
pub const MAX_FOV: f64 = 120.0;
/// Longest caption, in characters. A longer one is cut.
pub const MAX_NOTICE_CHARS: usize = 200;
/// Most groups of parameters kept folded shut.
pub const MAX_COLLAPSED: usize = 512;
/// Longest feature reference kept as the selection.
pub const MAX_SELECTION_CHARS: usize = 128;
/// Longest persistent name kept in a highlight or a pick. A name is a
/// chain of where the face came from and grows with every copy and
/// boolean it went through.
pub const MAX_FACE_NAME_CHARS: usize = 4096;
/// The display quality's chordal tolerance, as a fraction of the model's
/// size, is clamped to this range: at 1e-7 of the size a circle the size
/// of the model needs more segments than the tessellator allows on an
/// edge (it would be coarsened straight back), and coarser than a tenth
/// is a mesh that is not the model.
pub const MIN_CHORDAL: f64 = 1e-7;
pub const MAX_CHORDAL: f64 = 1e-1;
/// The most see-through the bodies get: nearly clear, with a tenth of the
/// front surface left so that it is still there.
pub const MAX_TRANSPARENCY: f64 = 0.9;
/// The display quality's triangle budget is clamped to this range.
pub const MIN_TRIANGLES: u64 = 1_000;
pub const MAX_TRIANGLES: u64 = 1 << 26;
/// The display quality's angle criterion, in degrees, is clamped to this:
/// above it a chord may turn a circle by more than a quarter, which is no
/// criterion a picture needs. Zero (or less) is no angle criterion.
pub const MAX_ANGLE_DEGREES: f64 = 90.0;

/// How finely the viewport's mesh follows the model: the display rule of
/// `craft3d_engine::SceneQuality::Display`, as the shell asks for it. The
/// other pictures (`ui.screenshot` follows this; `view.render`, previews
/// and `file.export` keep their own tolerance) are not affected.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct DisplayQuality {
    /// The chordal tolerance as a fraction of the model's size (the side
    /// of a square with the area of the quick first picture's mesh, which
    /// is the bodies' total area within that picture's tolerance): the default,
    /// `DISPLAY_RELATIVE_TOLERANCE`, keeps a circle round at any zoom the
    /// developer is likely to use.
    pub chordal: f64,
    /// The most triangles the viewport's scene may have: the mesh is
    /// coarsened until it fits, but never beyond the quick first picture's
    /// tolerance (1e-3 of the diagonal of the model's box).
    pub max_triangles: u64,
    /// The angle criterion beside the chordal tolerance, in degrees: no
    /// chord of an edge turns its tangent, and no side of a triangle the
    /// surface's normal, by more than this (unless it is shorter than the
    /// tolerance), so a small hole stays round. 0 for none. The default is
    /// `DISPLAY_MAX_ANGLE`.
    pub angle: f64,
}

impl Default for DisplayQuality {
    fn default() -> Self {
        Self {
            chordal: DISPLAY_RELATIVE_TOLERANCE,
            max_triangles: u64::try_from(DISPLAY_MAX_TRIANGLES).unwrap_or(u64::MAX),
            angle: DISPLAY_MAX_ANGLE_DEGREES,
        }
    }
}

impl DisplayQuality {
    /// The same quality with every number in range; a number that is not
    /// finite is an error.
    pub fn validated(mut self) -> Result<Self, String> {
        self.chordal = finite("quality.chordal", self.chordal)?.clamp(MIN_CHORDAL, MAX_CHORDAL);
        self.max_triangles = self.max_triangles.clamp(MIN_TRIANGLES, MAX_TRIANGLES);
        self.angle = finite("quality.angle", self.angle)?.clamp(0.0, MAX_ANGLE_DEGREES);
        Ok(self)
    }

    /// What the shell asks the engine for.
    pub fn scene_quality(self) -> SceneQuality {
        SceneQuality::Display {
            relative: self.chordal,
            max_triangles: usize::try_from(self.max_triangles).unwrap_or(usize::MAX),
            max_angle: (self.angle > 0.0).then(|| self.angle * (std::f64::consts::PI / 180.0)),
        }
    }
}

/// The named views, with the yaw and pitch each one is.
pub const VIEWS: [(&str, f64, f64); 7] = [
    // The direction `view.render` calls iso: (1, -1, 0.8) towards the eye.
    ("iso", -45.0, 29.496_208_496_566_43),
    ("front", -90.0, 0.0),
    ("back", 90.0, 0.0),
    ("right", 0.0, 0.0),
    ("left", 180.0, 0.0),
    ("top", -90.0, 90.0),
    ("bottom", -90.0, -90.0),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectionKind {
    #[default]
    Perspective,
    Orthographic,
}

/// An orbit camera: it looks at `target` from `distance` away.
///
/// `yaw` turns the eye about the world Z axis, counter-clockwise seen from
/// above, measured from +X; `pitch` lifts it above the XY plane. Both are in
/// degrees. The eye is at
/// `target + distance · (cos pitch · cos yaw, cos pitch · sin yaw, sin pitch)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct CameraState {
    pub target: [f64; 3],
    pub distance: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub projection: ProjectionKind,
    /// Vertical field of view in degrees. An orthographic camera shows what
    /// a perspective one with this angle shows at the target's depth.
    pub fov_y: f64,
    /// Frame everything again whenever the size of the model changes.
    /// Panning or zooming by hand switches it off; fitting switches it on.
    pub auto_fit: bool,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            target: [0.0; 3],
            distance: 100.0,
            yaw: -45.0,
            pitch: 29.496_208_496_566_43,
            projection: ProjectionKind::Perspective,
            fov_y: 30.0,
            auto_fit: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Panels {
    /// The feature tree, on the left.
    pub tree: bool,
    /// The status bar, at the bottom.
    pub status: bool,
    /// The bar that offers the documents a crashed 3DCraft left behind.
    /// Shown only while there are any; its Later button turns this off.
    pub recovery: bool,
    /// The feature tree's width in points. Only a drag on its split (or
    /// `ui.set`) changes it, never a long row in the panel; it is held to
    /// the tokens' range (`tree_min_width` to `tree_max_width`, at most
    /// `tree_max_share` of the window) when the panel is drawn.
    pub tree_width: f32,
    /// In the narrow layout (a phone, a small tablet, a narrow window) the
    /// side panel is a drawer over the viewport: open or not. Its button
    /// is at the left of the menu bar.
    pub drawer: bool,
    /// The studio look's options are open: its popover shows while the
    /// look is on (and no analysis). The Studio caret opens and closes
    /// it; switching the look on (the chip, or `ui.set` of `look.mode`)
    /// opens it; closed, the look stays as it is.
    pub look_options: bool,
    /// The analysis' options and legend are open: its popover shows while
    /// an analysis is on. The Analysis caret opens and closes it;
    /// switching an analysis on opens it.
    pub analysis_options: bool,
    /// The side panel's tab (`shell/sidepanel`). The tabs offered follow
    /// the document: a part has Features, Variables and Bodies; an
    /// assembly Assembly and Variables; a Jig in use (and an assembly
    /// with joints to set) Parameters first. A tab the document does not
    /// offer shows the nearest one it does (Features for Assembly and the
    /// other way round, else the first).
    pub side_tab: SideTab,
}

/// A tab of the side panel ([`Panels::side_tab`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideTab {
    #[default]
    Features,
    Variables,
    Bodies,
    Parameters,
    Assembly,
    /// What agents did to the document, newest first (the desktop app).
    Agent,
}

impl SideTab {
    /// As the tab bar writes it.
    pub fn label(self) -> &'static str {
        match self {
            SideTab::Features => "Features",
            SideTab::Variables => "Variables",
            SideTab::Bodies => "Bodies",
            SideTab::Parameters => "Parameters",
            SideTab::Assembly => "Assembly",
            SideTab::Agent => "Agent",
        }
    }
}

/// Most rows kept unfolded in the side panel's lists.
pub const MAX_UNFOLDED: usize = 512;
/// Longest filter text kept.
pub const MAX_FILTER_CHARS: usize = 200;

/// What the side panel shows besides its tab (`shell/sidepanel`): view
/// state, nothing of the document changes. `ui.set {"sidePanel":
/// {"hover": "F3"}}` shows a row's dependencies as the pointer on it
/// does; `{"menu": "F11"}` opens a row's context menu.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct SidePanelView {
    /// The Features tab's filter: rows whose name, kind or key value hold
    /// it (any case) are listed, with what they are nested in.
    pub filter: String,
    /// The Variables tab's filter, on names and formulas.
    pub variable_filter: String,
    /// The Bodies tab's filter, on names and materials.
    pub body_filter: String,
    /// The rows opened: a feature that has a sketch nested under it, a
    /// group of datums (by its first datum's id), a group of variables
    /// (`group:NAME`; groups are open unless listed under `folded`).
    pub unfolded: Vec<String>,
    /// The groups of variables folded shut, by name.
    pub folded: Vec<String>,
    /// The checks under the feature list are open (they fold to one line
    /// while something is selected, whatever this says).
    pub checks_open: bool,
    /// The Bodies tab also lists the bodies booleans used up (folded
    /// away by default).
    pub show_consumed: bool,
    /// The body whose card the Bodies tab shows, by its feature's id.
    pub body: Option<String>,
    /// The feature whose dependencies are shown, as the pointer on its
    /// row shows them (by id).
    pub hover: Option<String>,
    /// The feature whose context menu is open (by id).
    pub menu: Option<String>,
}

impl Default for SidePanelView {
    fn default() -> Self {
        Self {
            filter: String::new(),
            variable_filter: String::new(),
            body_filter: String::new(),
            unfolded: Vec::new(),
            folded: Vec::new(),
            checks_open: true,
            show_consumed: false,
            body: None,
            hover: None,
            menu: None,
        }
    }
}

impl SidePanelView {
    /// Holds what a patch may have made too long to bounds: the filters
    /// to [`MAX_FILTER_CHARS`], the lists to [`MAX_UNFOLDED`], the ids to
    /// [`MAX_SELECTION_CHARS`].
    pub fn bounded(&mut self) {
        for text in [
            &mut self.filter,
            &mut self.variable_filter,
            &mut self.body_filter,
        ] {
            if text.chars().count() > MAX_FILTER_CHARS {
                *text = text.chars().take(MAX_FILTER_CHARS).collect();
            }
        }
        self.unfolded.truncate(MAX_UNFOLDED);
        self.folded.truncate(MAX_UNFOLDED);
        for id in [&mut self.body, &mut self.hover, &mut self.menu] {
            if id
                .as_ref()
                .is_some_and(|id| id.chars().count() > MAX_SELECTION_CHARS)
            {
                *id = None;
            }
        }
    }

    /// Whether `key` is unfolded.
    pub fn is_open(&self, key: &str) -> bool {
        self.unfolded.iter().any(|k| k == key)
    }

    /// Opens or folds `key`.
    pub fn set_open(&mut self, key: &str, open: bool) {
        self.unfolded.retain(|k| k != key);
        if open && self.unfolded.len() < MAX_UNFOLDED {
            self.unfolded.push(key.to_string());
        }
    }
}

/// Work the window is doing on its engine thread while it keeps drawing:
/// a command, opening a file, meshing the model. Shown over the viewport
/// and in `ui.get` under `work`; `None` when the window is idle. Not part
/// of [`UiState`]: it cannot be set, only watched.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Work {
    /// What was asked for: the command (`file.open`), `batch of 3`, or
    /// `meshing` when only the picture is being made.
    pub label: String,
    /// What is being done right now: `running` before the first step,
    /// then `regenerating`, `measuring` or `meshing`.
    pub phase: String,
    /// The feature of the step (`F17`) and its name, when the step is one.
    pub feature: Option<String>,
    pub name: Option<String>,
    /// The step, counted from 1, of how many (the document's features); 0
    /// of 0 when the phase has no steps.
    pub step: usize,
    pub steps: usize,
    /// How long the work has run, in seconds.
    pub seconds: f64,
    /// What the work is about, when the host knows: the name of the file
    /// being opened (`welding-table`), for the status bar.
    pub subject: Option<String>,
}

impl Work {
    /// One line for the window: `file.open: regenerating J2 (17 of 20), 3 s`.
    pub fn describe(&self) -> String {
        let mut text = format!("{}: {}", self.label, self.phase);
        if let Some(what) = self.name.as_ref().or(self.feature.as_ref()) {
            text.push(' ');
            text.push_str(what);
        }
        if self.steps > 0 {
            text.push_str(&format!(" ({} of {})", self.step, self.steps));
        }
        if self.seconds.is_finite() && self.seconds >= 1.0 {
            text.push_str(&format!(", {:.0} s", self.seconds.floor()));
        }
        text
    }

    /// What the status bar says while the work runs, short and calm:
    /// `Opening welding-table… 41/82`, `Opening welding-table… meshing`,
    /// `Regenerating… 3/12`, `feature.extrude…`.
    pub fn status_text(&self) -> String {
        let doing = match (self.label.as_str(), self.subject.as_deref()) {
            ("file.open", Some(name)) => format!("Opening {name}"),
            ("file.open", None) => "Opening".to_string(),
            ("param.set" | "variable.set" | "edit.undo" | "edit.redo", _) => {
                "Regenerating".to_string()
            }
            ("meshing", _) => "Meshing".to_string(),
            (label, _) => label.to_string(),
        };
        let mut text = format!("{doing}\u{2026}");
        match self.phase.as_str() {
            "regenerating" if self.steps > 0 => {
                text.push_str(&format!(" {}/{}", self.step, self.steps));
            }
            "meshing" | "refining" if self.label != "meshing" => text.push_str(" meshing"),
            _ => {}
        }
        text
    }

    /// How far along the steps are, 0 to 1; `None` without steps.
    pub fn fraction(&self) -> Option<f32> {
        (self.steps > 0).then(|| (self.step as f32 / self.steps as f32).clamp(0.0, 1.0))
    }
}

/// One tab of the row above the viewport: an open document as the window
/// shows it, in the order of `file.list`. In `ui.get` under `tabs`; not
/// part of [`UiState`]: it follows the session and cannot be set (the
/// commands `file.activate` and `file.close` change it).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DocTab {
    /// The `document` number of `file.list` and `file.activate`.
    pub document: usize,
    /// What the tab says: the file's name without its extension, or
    /// `Untitled n` for a document that was never saved (n counts those
    /// from 1, in the order of the tabs).
    pub name: String,
    /// The path it was opened from or last saved to.
    pub path: Option<String>,
    /// It has changes that are not on disk: the tab is marked.
    pub dirty: bool,
    /// The document the window shows and the commands act on.
    pub active: bool,
    /// Closing it from its tab asks first (Save, Discard, Cancel): what
    /// `file.close` would refuse without `discard`.
    pub asks_to_close: bool,
    /// A published Jig opened to be used (`.3dcraftjig`): closing it after
    /// its parameters changed asks whether to save them as a model.
    pub jig_in_use: bool,
    /// An assembly (a `.3dcraftasm`, or one seen to be while it was the
    /// active document): the tab's icon says so.
    pub assembly: bool,
}

/// The question the window asks before it closes a document with unsaved
/// changes from its tab: Save, Discard or Cancel. In `ui.get` under
/// `closing` while it is asked; the person answers it in the window.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Closing {
    /// The document, by its number in `file.list`, and its tab's name and
    /// path when it was asked: a document that is not that one any more
    /// (an agent closed or saved it meanwhile) drops the question.
    pub document: usize,
    pub name: String,
    pub path: Option<String>,
    /// Asked because the window is closing (its close button, or
    /// `ui.quit {save: "ask"}`), not for one tab: the next document with
    /// unsaved changes is asked about after this one, and the
    /// application stops once none is left; Cancel keeps the window.
    pub quitting: bool,
    /// It is a Jig in use whose parameters were changed: the question is
    /// whether to save the result as a model (Save as model…, Close,
    /// Cancel); the Jig's own file is never written.
    pub jig: bool,
}

/// Whether AI agents may drive this window, and who does: what the start
/// screen's "Connect an AI agent" block, the Help menu's switch and the
/// status bar show, and `ui.get` reports under `agents`. The host fills it
/// in; it is read-only over the control channel: a request cannot turn the
/// channel it came over off (that is the person's switch), and nothing can
/// turn it on over a channel that is off. In the browser `available` is
/// false.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentsStatus {
    /// The host can let agents in (the desktop app).
    pub available: bool,
    /// The switch: agents are allowed, and the window listens.
    pub allowed: bool,
    /// Turned on by `--control PORT` on the command line.
    pub forced: bool,
    /// The port it listens on, while it does.
    pub port: Option<u16>,
    /// The clients connected now (an MCP bridge counts while it is heard
    /// from, or until it says goodbye).
    pub connected: usize,
    /// Who they are and what they did last.
    pub clients: Vec<AgentClient>,
    /// The folder agents may read and write files in: the one chosen
    /// with Change… (by default `Documents/3DCraft`), absolute; while the
    /// switch is on and the command line gave no roots.
    pub work_folder: Option<String>,
    /// The command line gave the roots (`--automation-read-root`,
    /// `--automation-write-root`): they win, Change… is off.
    pub command_line: bool,
    /// "Also the open document's folder" (on by default, remembered).
    pub document_folder: bool,
    /// The roots agents have now, in order (relative paths go beneath the
    /// first), and where each came from.
    pub roots: Vec<AgentRoot>,
    /// The questions the window asks the person about agents, oldest
    /// first: a client's working folder when it connects, a path outside
    /// every root. Only the person's click answers them: nothing on the
    /// control channel does (an agent approving its own access would
    /// defeat them).
    pub prompts: Vec<AgentPrompt>,
    /// Why the switch could not be turned on, or what went wrong around it.
    pub error: Option<String>,
    /// The person's home folder, to write paths beneath it with `~`; not
    /// reported.
    #[serde(skip)]
    pub home: Option<String>,
}

/// One client connected to the window.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentClient {
    /// The name its MCP client gave (`Claude Code`), else the one it gave
    /// when it authenticated (`MCP bridge (process 4242)`), else "control
    /// connection".
    pub name: String,
    /// Since when, in milliseconds since the Unix epoch.
    pub since_ms: u64,
    /// The command it ran last (`feature.edit`), and when.
    pub last_command: Option<String>,
    pub last_ms: u64,
    /// The folder it works in (its MCP client's working folder, which the
    /// bridge sends), absolute.
    pub cwd: Option<String>,
}

/// A question about an agent's access, waiting for the person.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentPrompt {
    pub id: u64,
    /// `connect` (the client's working folder), `read` or `write` (a path
    /// a command wants, outside every root).
    pub kind: String,
    /// The client, by the name it gave.
    pub client: String,
    /// The full path: the folder for `connect`, the file otherwise.
    pub path: String,
    /// The folder: what "Always allow" grants (for `connect`, the path
    /// itself); shown on a line of its own.
    pub folder: String,
    /// The file's name, shown on a line of its own (empty for `connect`).
    pub file_name: String,
    /// The command that asks (`file.save`), for `read` and `write`.
    pub command: Option<String>,
}

/// The person's answer to an [`AgentPrompt`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PromptAnswer {
    /// Allow the folder, remembered (connect), or allow this folder for
    /// the agents from now on (a path).
    Folder,
    /// The folder for this run only (connect).
    Session,
    /// This one command only (a path).
    Once,
    /// No.
    Deny,
}

/// One folder agents may read or write files in.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentRoot {
    pub path: String,
    /// `chosen` (the switch's folder), `allowed` (a folder the person
    /// allowed, remembered), `session` (allowed for this run), `once` (for
    /// one command), `document` (the open document's folder) or
    /// `command line`.
    pub from: String,
    pub read: bool,
    pub write: bool,
}

/// What the viewport draws besides the bodies' faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Show {
    /// Datum planes, axes and points (shown by default).
    pub datums: bool,
    /// The sketches: their lines and points (shown by default).
    pub sketches: bool,
    /// The bodies' edges, the outline of a section's cap with them
    /// (shown by default). A highlighted or selected edge or vertex is
    /// drawn either way: it points at something, it is not a line of the
    /// model.
    pub edges: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            datums: true,
            sketches: true,
            edges: true,
        }
    }
}

/// The windows of the Help menu that are open: About 3DCraft and the
/// keyboard shortcuts (F1). `ui.set {"dialogs": {"shortcuts": true}}`
/// opens one, for an agent that shows the person around.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Dialogs {
    pub about: bool,
    pub shortcuts: bool,
    /// The agents popover over the status bar's agents pill (2026-11-02):
    /// the same panel as the start screen's "Connect an AI agent".
    pub agents: bool,
}

/// The history player under the feature list: playing through the
/// timeline step by step, how fast, whether it starts again at the end,
/// and whether the camera turns slowly meanwhile. View state, like the
/// timeline itself (`view.scrub`): `ui.set {"timeline": {"playing":
/// true}}` starts a replay for the person watching. Not kept per tab:
/// switching tabs stops it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Playback {
    /// Playing: the next step follows once the step shown is built and
    /// has been shown long enough ([`PLAYER_DWELL_SECONDS`] / `speed`).
    pub playing: bool,
    /// One of [`PLAYER_SPEEDS`].
    pub speed: f64,
    /// Starts again at the first step after the last.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// The camera turns slowly about the vertical while playing.
    pub orbit: bool,
    /// Plays the motion instead of the history: the timeline `t` of the
    /// document runs from 0 to 1 (`view.motion`) in
    /// [`MOTION_SECONDS`] / `speed`, a frame as soon as the one before is
    /// shown. Only for a document with a timeline (a variable `t` that is
    /// a plain number).
    pub motion: bool,
    /// With `motion`: back and forth, from 1 back to 0 and on (with `loop`
    /// for ever, else once there and back).
    pub bounce: bool,
    /// With `motion`: the bodies that overlap at the `t` shown are drawn
    /// red (`view.motion {collisions}`).
    pub collisions: bool,
    /// How the viewport draws while the motion plays: `auto` lighter (one
    /// frame, no depth of field, no ambient occlusion or screen-space
    /// reflections), `full` as when it stands
    /// still. Pictures and recordings are full either way
    /// (docs/design/motion.md › *The player*).
    pub play_quality: PlayQuality,
}

/// How the viewport draws while a motion plays ([`Playback::play_quality`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayQuality {
    /// Lighter while it plays (the studio look in one frame, without its
    /// depth of field, ambient occlusion and screen-space reflections);
    /// full again once it stops.
    #[default]
    Auto,
    /// The full quality while it plays too.
    Full,
}

/// How long the motion takes from `t` = 0 to 1 at speed 1, in seconds.
pub const MOTION_SECONDS: f64 = 4.0;

/// The speeds the player offers.
pub const PLAYER_SPEEDS: [f64; 3] = [0.5, 1.0, 2.0];

/// How long each step is shown at least, at speed 1, in seconds.
pub const PLAYER_DWELL_SECONDS: f64 = 0.8;

impl Default for Playback {
    fn default() -> Self {
        Self {
            playing: false,
            speed: 1.0,
            looping: false,
            orbit: false,
            motion: false,
            bounce: false,
            collisions: false,
            play_quality: PlayQuality::Auto,
        }
    }
}

impl Playback {
    /// The same playback with a speed the player offers; any other speed
    /// is an error.
    pub fn validated(self) -> Result<Self, String> {
        if PLAYER_SPEEDS.contains(&self.speed) {
            Ok(self)
        } else {
            Err(format!(
                "`timeline.speed` must be one of 0.5, 1 and 2, not {}",
                self.speed
            ))
        }
    }

    /// The speed after `self.speed` in [`PLAYER_SPEEDS`], round.
    pub fn next_speed(self) -> f64 {
        let at = PLAYER_SPEEDS
            .iter()
            .position(|s| *s == self.speed)
            .map_or(1, |i| i.saturating_add(1) % PLAYER_SPEEDS.len());
        PLAYER_SPEEDS.get(at).copied().unwrap_or(1.0)
    }

    /// How long a step is shown at least, in seconds.
    pub fn dwell(self) -> f64 {
        PLAYER_DWELL_SECONDS / self.speed.max(0.25)
    }
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            tree: true,
            status: true,
            recovery: true,
            tree_width: crate::theme::Tokens::default().tree_width,
            drawer: false,
            look_options: true,
            analysis_options: true,
            side_tab: SideTab::Features,
        }
    }
}

/// A face of a body, the way `document.inspect` says it: the feature that
/// made the body and the face's persistent name.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct FaceRef {
    /// The feature of the body, by id (`F3`) or by name. `None` looks in
    /// every body, which works while only one has a face of that name.
    pub feature: Option<String>,
    /// The persistent name (`faceNames` of `document.inspect`), or one of
    /// the face's aliases.
    pub face: String,
}

/// An edge of a body, the way `document.edges` says it: by its own
/// persistent name where it has one, or by the two faces it lies between.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct EdgeRef {
    /// The feature of the body, by id (`F3`) or by name. `None` looks in
    /// every body, which works while only one has such an edge.
    pub feature: Option<String>,
    /// The edge's own persistent name (`name` of `document.edges`).
    pub edge: Option<String>,
    /// Without a name of its own: the names of the two faces it lies
    /// between, as `document.edges` takes them under `between` (the same
    /// name twice for a seam). Every edge between them is meant.
    pub between: Vec<String>,
}

/// A vertex of a body: by its own persistent name where it has one, or by
/// the faces that meet in it.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct VertexRef {
    /// The feature of the body, by id (`F3`) or by name. `None` looks in
    /// every body, which works while only one has such a vertex.
    pub feature: Option<String>,
    /// The vertex's own persistent name (`name` under `vertices` of
    /// `document.edges`).
    pub vertex: Option<String>,
    /// Without a name of its own: names of faces that meet in it. Every
    /// vertex in which all of them meet is meant.
    pub corner: Vec<String>,
}

/// Most face names a [`VertexRef`] keeps.
pub const MAX_CORNER_FACES: usize = craft3d_engine::MAX_CORNER_FACES;

/// The section plane of the viewport: a plane that cuts the picture open,
/// to look inside a shell, a pocket or a cavity. A view, like the camera:
/// the document and its bodies do not change, and nothing is regenerated.
///
/// The plane is `plane` (a base plane, a datum plane, or a flat face by
/// its name) or `point` and `normal`, moved by `offset` along its normal.
/// What lies on the side the normal points to is not drawn (a face's
/// normal points out of its body); `flip` keeps that half and cuts the
/// other away.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct SectionState {
    /// Whether the picture is cut.
    pub on: bool,
    /// `xy`, `yz` or `zx`, a datum plane by its feature's id (`F5`) or
    /// name, or a flat face by its persistent name. When the section is
    /// switched on with no plane at all, it is `xy`.
    pub plane: Option<String>,
    /// Which body has the face, by the id of its feature or its name;
    /// needed only when several bodies have a face of that name.
    pub feature: Option<String>,
    /// A plane given directly, in place of `plane`: a point of it (the
    /// origin when only the normal is given) and its normal.
    pub point: Option<[f64; 3]>,
    pub normal: Option<[f64; 3]>,
    /// How far the plane is moved along its normal, in model units.
    pub offset: f64,
    /// Keep the other half.
    pub flip: bool,
}

impl SectionState {
    /// The same section with everything in range; a number that is not
    /// finite is an error.
    pub fn validated(mut self) -> Result<Self, String> {
        self.offset = finite("section.offset", self.offset)?.clamp(-MAX_EXTENT, MAX_EXTENT);
        for (name, triple) in [("point", &mut self.point), ("normal", &mut self.normal)] {
            if let Some(triple) = triple {
                for value in triple.iter_mut() {
                    *value =
                        finite(&format!("section.{name}"), *value)?.clamp(-MAX_EXTENT, MAX_EXTENT);
                }
            }
        }
        self.plane = self
            .plane
            .map(|name| cut(&name, MAX_FACE_NAME_CHARS))
            .filter(|name| !name.is_empty());
        self.feature = self
            .feature
            .map(|id| cut(&id, MAX_SELECTION_CHARS))
            .filter(|id| !id.is_empty());
        if self.normal.is_some() && self.point.is_none() && self.plane.is_none() {
            self.point = Some([0.0; 3]);
        }
        if self.on && self.plane.is_none() && self.normal.is_none() {
            self.plane = Some("xy".to_string());
            self.point = None;
        }
        Ok(self)
    }

    /// What the engine resolves ([`craft3d_engine::Session::section_plane`]).
    pub fn spec(&self) -> craft3d_engine::SectionSpec {
        craft3d_engine::SectionSpec {
            plane: self.plane.clone(),
            feature: self.feature.clone(),
            point: self.point,
            normal: self.normal,
            offset: self.offset,
            flip: self.flip,
        }
    }
}

/// What a click was on: a face, or an edge or a vertex near the position,
/// or the cap of the section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PickedKind {
    Face,
    Edge,
    Vertex,
    /// The cap of the section: `feature` and `name` are then the body
    /// that is cut there and `point` the point of the plane; there is no
    /// face.
    Section,
}

/// The edge or the vertex a click was on.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PickedEntity {
    /// The feature of its body (`F3`) and that feature's name.
    pub feature: String,
    pub body: String,
    /// Its own persistent name, when it has one.
    pub name: Option<String>,
    /// Its place in `document.edges`: among the edges, or `index` of a
    /// vertex.
    pub index: u64,
    /// The names of the faces an edge lies between, or that meet in a
    /// vertex.
    pub faces: Vec<String>,
    /// The point of an edge nearest to the click, or the vertex, in model
    /// coordinates.
    pub point: [f64; 3],
}

/// The last click in the viewport, or the last `ui.pick` that asked to be
/// taken as one: where it was and what was under it. An agent reads here
/// what the developer pointed at.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct Picked {
    /// How many picks there have been in this window, this one included:
    /// it moves with every click, also one that hits the same face or
    /// nothing.
    pub serial: u64,
    /// Where, in viewport pixels from the top-left corner.
    pub x: f64,
    pub y: f64,
    /// The feature of the body under it (`F3`) and that feature's name;
    /// `None` when nothing was under it.
    pub feature: Option<String>,
    pub name: Option<String>,
    /// The face's persistent name, as `document.inspect` lists it.
    pub face: Option<String>,
    /// The face's place in `faceNames` of its body.
    pub face_index: Option<u64>,
    /// The point under it, in model coordinates.
    pub point: Option<[f64; 3]>,
    /// What the click was on: a vertex when one was near, otherwise an
    /// edge when one was near, otherwise the face under it; `None` for a
    /// click on nothing. The fields above are the face under the click
    /// either way.
    pub kind: Option<PickedKind>,
    /// The edge, when the click was on one.
    pub edge: Option<PickedEntity>,
    /// The vertex, when the click was on one.
    pub vertex: Option<PickedEntity>,
}

/// Everything the shell remembers that is not in a document.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct UiState {
    pub camera: CameraState,
    pub panels: Panels,
    /// The feature highlighted in the tree and the viewport, by id (`F2`).
    pub selected_feature: Option<String>,
    /// A one-line caption shown over the viewport, for narrated demos.
    pub notice: String,
    /// How finely the viewport's mesh follows the model.
    pub quality: DisplayQuality,
    /// The face shown highlighted in the viewport: the one that was
    /// clicked last, or the one an agent pointed at (`ui.highlight`, or
    /// `ui.set`). It is dropped when no body has a face of that name any
    /// more.
    pub highlight: Option<FaceRef>,
    /// The edge shown highlighted in the viewport: the one that was
    /// clicked last, or the one an agent pointed at (`ui.highlight`, or
    /// `ui.set`). Dropped when no body has such an edge any more.
    pub highlight_edge: Option<EdgeRef>,
    /// The vertex shown highlighted, likewise.
    pub highlight_vertex: Option<VertexRef>,
    /// How see-through the bodies are drawn, 0 (opaque, the default) to
    /// [`MAX_TRANSPARENCY`]: the front surface is blended over the one
    /// behind it, so that inner faces, holes and cavities show
    /// (`craft3d_render::render`, *See-through*). The slider in the
    /// viewport sets it; `ui.set` too.
    pub transparency: f64,
    /// The plane the viewport's picture is cut open with. The control near
    /// the view cube sets it; `ui.set` too. A plane whose name no body or
    /// feature has any more switches the section off (`ui.get` says why
    /// under `section`).
    pub section: SectionState,
    /// The last click in the viewport. Read-only over the control channel:
    /// `ui.pick` with `select` makes one.
    pub picked: Option<Picked>,
    /// What the viewport draws besides the bodies' faces:
    /// `ui.set {"show": {"datums": false}}` hides the datums,
    /// `{"show": {"sketches": false}}` the sketches, `{"show": {"edges":
    /// false}}` the bodies' edges.
    pub show: Show,
    /// The Help menu's windows that are open.
    pub dialogs: Dialogs,
    /// The history player.
    pub timeline: Playback,
    /// The Parameters tab of the side panel (`shell/params.rs`).
    pub parameters: ParametersView,
    /// The export dialog: open or not, and what it has chosen (kept
    /// while it is closed, offered again next time).
    pub export: ExportState,
    /// How the bodies are lit: the plain look or the studio look. View
    /// state; the document does not change.
    pub look: LookState,
    /// A surface analysis drawn in place of the shading (zebra stripes,
    /// highlight lines, a curvature map, draft) and curvature combs. View
    /// state; the document does not change.
    pub analysis: AnalysisState,
    /// Light or dark panels, or what the system prefers (the default). A
    /// preference of the window, never part of a document or of a picture
    /// (`view.render`, `ui.screenshot`'s viewport): the viewport is not
    /// themed. `ui.set {"appearance": "dark"}` sets it.
    pub appearance: Appearance,
    /// The side panel's filters, folds, hover and menu
    /// (`shell/sidepanel`).
    pub side_panel: SidePanelView,
}

/// Which colours the panels have (`theme::Palette`): the system's choice
/// (the desktop's light or dark setting; in the browser
/// `prefers-color-scheme`), light, or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// Whether the panels are dark, given what the system prefers
    /// (`egui::Context::system_theme`: eframe reads it from the desktop and
    /// from the browser's `prefers-color-scheme`). A system that says
    /// nothing (tests, a headless pass) is light, so pictures made without
    /// a window stay as they were.
    pub fn is_dark(self, system: Option<egui::Theme>) -> bool {
        match self {
            Appearance::Light => false,
            Appearance::Dark => true,
            Appearance::System => system == Some(egui::Theme::Dark),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }
}

/// The surface analysis of the viewport (`craft3d_engine::analysis`, the
/// same fields as `view.render`'s `analysis`): `mode` none (the default),
/// zebra, highlight, curvature or draft, with its options, and combs.
/// `ui.set {"analysis": {"mode": "zebra"}}` and the Analysis toggle of the
/// display bar switch it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct AnalysisState {
    pub mode: String,
    pub stripes: f64,
    /// Degrees.
    pub direction: f64,
    pub width: f64,
    pub offset: f64,
    pub pull: [f64; 3],
    /// Degrees.
    pub min_draft: f64,
    pub curvature: String,
    pub range: Option<[f64; 2]>,
    /// As `analysis.combs` takes it, or `None` for none.
    pub combs: Option<serde_json::Value>,
    pub legend: bool,
}

impl Default for AnalysisState {
    fn default() -> Self {
        AnalysisState::of(&craft3d_engine::analysis::AnalysisSpec::default())
    }
}

impl AnalysisState {
    /// The state of a parsed analysis.
    pub fn of(spec: &craft3d_engine::analysis::AnalysisSpec) -> AnalysisState {
        let json = spec.json();
        AnalysisState {
            mode: json
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("none")
                .to_string(),
            stripes: spec.stripes,
            direction: spec.direction,
            width: spec.width,
            offset: spec.offset,
            pull: [spec.pull.x, spec.pull.y, spec.pull.z],
            min_draft: spec.min_draft,
            curvature: spec.curvature.name().to_string(),
            range: spec.range,
            combs: json.get("combs").filter(|c| !c.is_null()).cloned(),
            legend: spec.legend,
        }
    }

    /// The analysis this state says, checked.
    pub fn spec(&self) -> Result<craft3d_engine::analysis::AnalysisSpec, String> {
        let value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        craft3d_engine::analysis::AnalysisSpec::patched(
            &craft3d_engine::analysis::AnalysisSpec::default(),
            &value,
        )
    }

    pub fn is_on(&self) -> bool {
        self.mode != "none" || self.combs.is_some()
    }
}

/// How the viewport lights the bodies (`craft3d_engine::look`, the same
/// fields as `view.render`'s `look`): `mode` plain (the default) or
/// studio, with the studio look's environment and options. `ui.set
/// {"look": {"mode": "studio"}}` and the Studio toggle of the camera
/// plate switch it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LookState {
    pub mode: String,
    pub environment: String,
    pub exposure: f64,
    pub rotation: f64,
    pub ground: bool,
    pub ao: bool,
    pub shadows: bool,
    pub tonemap: String,
    pub light: Option<[f64; 2]>,
    /// `environment` (the default) or `plain`: the theme's background
    /// colour behind a model the environment lights.
    pub background: String,
    /// Layer 2 (`craft3d_render::studio::polish`).
    pub reflections: bool,
    pub contact_shadows: bool,
    pub specular_shadow: bool,
    pub ground_reflection: f64,
    pub bloom: f64,
    /// `{fstop, focus}` as `look.dof` takes it, or `None` for no lens.
    pub dof: Option<serde_json::Value>,
    /// The frames averaged while the camera stands still (`None`:
    /// `craft3d_engine::look::VIEWPORT_ACCUMULATE`).
    pub accumulate: Option<u32>,
    /// How the studio look draws the bodies' edges ([`EDGE_MODES`]):
    /// `soft` (the default) fades them where the lens blurs the model,
    /// `sharp` draws them as the plain look does, `off` leaves them out.
    /// The window's own: `view.render` draws no edges, so the engine's
    /// look does not have it.
    pub edges: String,
}

/// The ways the studio look draws the bodies' edges (`look.edges`).
pub const EDGE_MODES: [&str; 3] = ["soft", "sharp", "off"];

/// How a look draws the bodies' edges ([`LookState::edge_mode`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeMode {
    /// Crisp over the picture, as the plain look draws them.
    Sharp,
    /// Faded where the lens blurs the model
    /// (`viewport::EdgeBlur`); crisp without a lens.
    Soft,
    /// Not drawn.
    Off,
}

impl Default for LookState {
    fn default() -> Self {
        LookState::of(&craft3d_engine::look::LookSpec::default())
    }
}

impl LookState {
    /// The state of a parsed look.
    pub fn of(spec: &craft3d_engine::look::LookSpec) -> LookState {
        let look = &spec.look;
        LookState {
            mode: if spec.studio { "studio" } else { "plain" }.to_string(),
            environment: spec.environment.clone(),
            exposure: look.exposure,
            rotation: look.rotation,
            ground: look.ground,
            ao: look.ao,
            shadows: look.shadows,
            tonemap: look.tonemap.name().to_string(),
            light: look.light,
            background: if look.plain_background {
                "plain"
            } else {
                "environment"
            }
            .to_string(),
            reflections: look.ssr,
            contact_shadows: look.contact_shadows,
            specular_shadow: look.specular_shadow,
            ground_reflection: look.ground_reflection,
            bloom: look.bloom,
            dof: spec.json().get("dof").filter(|v| !v.is_null()).cloned(),
            accumulate: spec.accumulate,
            edges: EDGE_MODES[0].to_string(),
        }
    }

    /// The look this state says, checked: an unknown mode, environment or
    /// tone curve is an error. (`edges` is the window's, not the
    /// engine's: see [`Self::checked`].)
    pub fn spec(&self) -> Result<craft3d_engine::look::LookSpec, String> {
        let mut value = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let Some(map) = value.as_object_mut() {
            map.remove("edges");
        }
        craft3d_engine::look::LookSpec::patched(&craft3d_engine::look::LookSpec::default(), &value)
    }

    /// This look checked and as parsed (clamped, turned within 0 to 360
    /// degrees), its `edges` kept: an unknown mode, environment, tone
    /// curve or way of drawing the edges is an error.
    pub fn checked(&self) -> Result<LookState, String> {
        if !EDGE_MODES.contains(&self.edges.as_str()) {
            let edges: String = self.edges.chars().take(40).collect();
            return Err(format!(
                "`look.edges` `{edges}` is not one of {}",
                EDGE_MODES.join(", ")
            ));
        }
        let spec = self.spec()?;
        Ok(LookState {
            edges: self.edges.clone(),
            ..LookState::of(&spec)
        })
    }

    /// How the bodies' edges are drawn in this look: as ever in the plain
    /// look, as `edges` says in the studio look.
    pub fn edge_mode(&self) -> EdgeMode {
        match (self.is_studio(), self.edges.as_str()) {
            (true, "off") => EdgeMode::Off,
            (true, "soft") => EdgeMode::Soft,
            _ => EdgeMode::Sharp,
        }
    }

    pub fn is_studio(&self) -> bool {
        self.mode == "studio"
    }
}

/// The export dialog (File › Export…, `shell/dialogs.rs`): what goes into
/// the file, its format and the options, before the system's save dialog
/// names the file. `ui.set {"export": {"open": true}}` opens it as the
/// menu does; `what` and `format` are refused when the document does not
/// offer them. Exporting itself (the *Export…* button) is the person's.
/// While it is closed, the fields are what the last export that was
/// written chose (a cancel or a failure keeps nothing), or what `ui.set`
/// gave for the next opening (`what` when the document offers it).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ExportState {
    pub open: bool,
    pub what: crate::files::ExportWhat,
    pub format: crate::files::ExportFormat,
    /// A mesh's quality (STL, 3MF, OBJ).
    pub quality: crate::files::ExportQuality,
    /// A drawing of the model as seen (the model to DXF or SVG).
    pub drawing: crate::files::DrawingPlan,
    /// A sheet's width and the gap between its outlines, as typed (a
    /// set's sheet, every part's flat patterns); empty for no limit and
    /// 5 mm.
    pub width: String,
    pub gap: String,
    /// The file's name: the one the system's save dialog proposes, or
    /// where it has none (the browser without file dialogs) the one the
    /// person types; its extension follows the format.
    pub name: String,
}

impl ExportState {
    /// How to export, as `file.export`'s parameters are made from it.
    pub fn plan(&self) -> crate::files::ExportPlan {
        crate::files::ExportPlan {
            what: self.what.clone(),
            quality: self.quality,
            drawing: self.drawing,
            width: self.width.clone(),
            gap: self.gap.clone(),
        }
    }
}

/// The Parameters tab beside the feature list: whether it is the one
/// shown (a Jig opens on it; each document keeps its choice), and the
/// groups of parameters that are folded shut. `ui.set {"parameters":
/// {"open": true}}` shows it.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ParametersView {
    pub open: bool,
    pub collapsed: Vec<String>,
}

/// What each document tab keeps of the view: the fields of [`UiState`]
/// that are about the document shown, so that orbiting or cutting one
/// document leaves the others as they were. The window holds the active
/// tab's in [`UiState`] (what `ui.get` and `ui.set` read and change) and
/// the others aside, by document; switching tabs swaps them. A document
/// opened or created starts with the default: framed whole (`autoFit`),
/// the iso view, perspective, no section, opaque, nothing highlighted.
///
/// Not kept per tab, because they are about the window or the person's
/// taste rather than a document: the panels, the caption, the display
/// quality and what is shown besides the bodies (`show`, which the web
/// build keeps as a preference).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct TabView {
    pub camera: CameraState,
    pub section: SectionState,
    pub transparency: f64,
    pub selected_feature: Option<String>,
    pub highlight: Option<FaceRef>,
    pub highlight_edge: Option<EdgeRef>,
    pub highlight_vertex: Option<VertexRef>,
    pub picked: Option<Picked>,
}

impl TabView {
    /// The tab's part of `state`.
    pub fn of(state: &UiState) -> Self {
        Self {
            camera: state.camera,
            section: state.section.clone(),
            transparency: state.transparency,
            selected_feature: state.selected_feature.clone(),
            highlight: state.highlight.clone(),
            highlight_edge: state.highlight_edge.clone(),
            highlight_vertex: state.highlight_vertex.clone(),
            picked: state.picked.clone(),
        }
    }

    /// Puts the tab's view into `state`; the rest of it stays.
    pub fn put(self, state: &mut UiState) {
        let Self {
            camera,
            section,
            transparency,
            selected_feature,
            highlight,
            highlight_edge,
            highlight_vertex,
            picked,
        } = self;
        state.camera = camera;
        state.section = section;
        state.transparency = transparency;
        state.selected_feature = selected_feature;
        state.highlight = highlight;
        state.highlight_edge = highlight_edge;
        state.highlight_vertex = highlight_vertex;
        state.picked = picked;
    }
}

fn finite(name: &str, value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("`{name}` must be a finite number"))
    }
}

/// Brings an angle into (-180, 180].
fn wrap_degrees(angle: f64) -> f64 {
    let wrapped = angle.rem_euclid(360.0);
    if wrapped > 180.0 {
        wrapped - 360.0
    } else {
        wrapped
    }
}

fn cut(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// One line: control characters (a newline in a caption) become spaces.
fn one_line(text: &str, max_chars: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max_chars)
        .collect()
}

impl CameraState {
    /// The same camera with every number in range: angles wrapped and
    /// clamped, lengths clamped. A number that is not finite is an error,
    /// because there is nothing sensible to clamp it to.
    pub fn validated(mut self) -> Result<Self, String> {
        for (axis, value) in ["x", "y", "z"].iter().zip(self.target.iter_mut()) {
            *value =
                finite(&format!("camera.target.{axis}"), *value)?.clamp(-MAX_EXTENT, MAX_EXTENT);
        }
        self.distance = finite("camera.distance", self.distance)?.clamp(MIN_DISTANCE, MAX_EXTENT);
        self.yaw = wrap_degrees(finite("camera.yaw", self.yaw)?);
        self.pitch = finite("camera.pitch", self.pitch)?.clamp(-90.0, 90.0);
        self.fov_y = finite("camera.fovY", self.fov_y)?.clamp(MIN_FOV, MAX_FOV);
        Ok(self)
    }

    /// Turns the camera to a named view; the target and distance stay.
    pub fn set_view(&mut self, name: &str) -> Result<(), String> {
        let Some(&(_, yaw, pitch)) = VIEWS.iter().find(|(view, ..)| *view == name) else {
            let names: Vec<&str> = VIEWS.iter().map(|(view, ..)| *view).collect();
            return Err(format!(
                "unknown view `{}`; use {}",
                cut(name, 40),
                names.join(", ")
            ));
        };
        self.yaw = yaw;
        self.pitch = pitch;
        Ok(())
    }
}

impl UiState {
    /// The same state with everything in range (see
    /// [`CameraState::validated`]); texts are cut to their limits.
    pub fn validated(mut self) -> Result<Self, String> {
        self.camera = self.camera.validated()?;
        self.quality = self.quality.validated()?;
        self.transparency = finite("transparency", self.transparency)?.clamp(0.0, MAX_TRANSPARENCY);
        self.section = self.section.validated()?;
        self.timeline = self.timeline.validated()?;
        self.notice = one_line(&self.notice, MAX_NOTICE_CHARS);
        for text in [
            &mut self.export.width,
            &mut self.export.gap,
            &mut self.export.name,
        ] {
            *text = one_line(text, crate::files::MAX_EXPORT_TEXT);
        }
        self.parameters.collapsed.truncate(MAX_COLLAPSED);
        for group in &mut self.parameters.collapsed {
            *group = one_line(group, MAX_SELECTION_CHARS);
        }
        self.selected_feature = self
            .selected_feature
            .map(|id| cut(&id, MAX_SELECTION_CHARS))
            .filter(|id| !id.is_empty());
        self.highlight = self
            .highlight
            .map(|face| FaceRef {
                feature: face
                    .feature
                    .map(|id| cut(&id, MAX_SELECTION_CHARS))
                    .filter(|id| !id.is_empty()),
                face: cut(&face.face, MAX_FACE_NAME_CHARS),
            })
            .filter(|face| !face.face.is_empty());
        self.highlight_edge = self
            .highlight_edge
            .map(|edge| EdgeRef {
                feature: edge
                    .feature
                    .map(|id| cut(&id, MAX_SELECTION_CHARS))
                    .filter(|id| !id.is_empty()),
                edge: edge
                    .edge
                    .map(|name| cut(&name, MAX_FACE_NAME_CHARS))
                    .filter(|name| !name.is_empty()),
                between: edge
                    .between
                    .iter()
                    .take(2)
                    .map(|name| cut(name, MAX_FACE_NAME_CHARS))
                    .collect(),
            })
            .filter(|edge| edge.edge.is_some() || !edge.between.is_empty());
        self.highlight_vertex = self
            .highlight_vertex
            .map(|vertex| VertexRef {
                feature: vertex
                    .feature
                    .map(|id| cut(&id, MAX_SELECTION_CHARS))
                    .filter(|id| !id.is_empty()),
                vertex: vertex
                    .vertex
                    .map(|name| cut(&name, MAX_FACE_NAME_CHARS))
                    .filter(|name| !name.is_empty()),
                corner: vertex
                    .corner
                    .iter()
                    .take(MAX_CORNER_FACES)
                    .map(|name| cut(name, MAX_FACE_NAME_CHARS))
                    .collect(),
            })
            .filter(|vertex| vertex.vertex.is_some() || !vertex.corner.is_empty());
        Ok(self)
    }

    /// Changes the fields `patch` names and leaves the rest: the body of
    /// `ui.set`. `patch` is a JSON object shaped like the state itself,
    /// plus `"view"` for a named view. An unknown field or a value of the
    /// wrong type is an error, and then nothing has changed.
    pub fn apply_patch(&mut self, patch: &Value) -> Result<(), String> {
        let Value::Object(patch) = patch else {
            return Err("the state to set must be a JSON object".to_string());
        };
        // The view first: named together with explicit angles, the angles
        // win, as the more specific request.
        let mut turned = self.clone();
        if let Some(view) = patch.get("view") {
            let Value::String(name) = view else {
                return Err("`view` must be a string".to_string());
            };
            turned.camera.set_view(name)?;
        }
        if patch.contains_key("picked") {
            return Err(
                "`picked` is what was clicked in the viewport and cannot be set; ui.pick with `select` makes a pick, ui.highlight (or `highlight` here) points at a face"
                    .to_string(),
            );
        }
        let mut merged = serde_json::to_value(&turned).map_err(|e| e.to_string())?;
        let mut moved_by_hand = false;
        for (key, value) in patch.iter().filter(|(key, _)| *key != "view") {
            if key == "camera"
                && let Value::Object(camera) = value
            {
                moved_by_hand = (camera.contains_key("target") || camera.contains_key("distance"))
                    && !camera.contains_key("autoFit");
            }
            if key == "section"
                && let (Value::Object(given), Value::Object(map)) = (value, &mut merged)
                && let Some(Value::Object(section)) = map.get_mut("section")
            {
                // Another plane, not the one before with a field changed:
                // a plane by its name replaces a plane by point and
                // normal and the other way round, and a face without
                // `feature` is looked for in every body.
                let names = |key: &str| given.contains_key(key);
                if names("plane") {
                    for other in ["point", "normal", "feature"] {
                        if !names(other) {
                            section.insert(other.to_string(), Value::Null);
                        }
                    }
                } else if names("point") || names("normal") {
                    for other in ["plane", "feature"] {
                        section.insert(other.to_string(), Value::Null);
                    }
                }
            }
            if key == "highlight" || key == "highlightEdge" || key == "highlightVertex" {
                // Another face, not the same one with a field changed: a
                // highlight without `feature` looks in every body.
                if let Value::Object(map) = &mut merged {
                    map.insert(key.clone(), value.clone());
                }
            } else {
                merge(&mut merged, key, value);
            }
            // Another choice of what to export, not the one before with a
            // field added (`{"set": …}` after `{"sheetMetal": …}`).
            if key == "export"
                && let (Some(what), Value::Object(map)) = (value.get("what"), &mut merged)
                && let Some(Value::Object(export)) = map.get_mut("export")
            {
                export.insert("what".to_string(), what.clone());
            }
        }
        let mut next: UiState = serde_json::from_value(merged).map_err(|e| e.to_string())?;
        // Switching the look or an analysis on opens its options, as its
        // chip does, unless the patch says otherwise.
        let given = |key: &str| {
            patch
                .get("panels")
                .and_then(Value::as_object)
                .is_some_and(|p| p.contains_key(key))
        };
        if !self.look.is_studio() && next.look.is_studio() && !given("lookOptions") {
            next.panels.look_options = true;
        }
        if !self.analysis.is_on() && next.analysis.is_on() && !given("analysisOptions") {
            next.panels.analysis_options = true;
        }
        if moved_by_hand {
            // Placing the camera by hand and having it refit at the next
            // edit would undo the request.
            next.camera.auto_fit = false;
        }
        next.side_panel.bounded();
        *self = next.validated()?;
        Ok(())
    }
}

/// Puts `value` at `key` of the object `into`; an object is merged key by
/// key into the object already there, anything else replaces it.
fn merge(into: &mut Value, key: &str, value: &Value) {
    let Value::Object(map) = into else {
        return;
    };
    match (map.get_mut(key), value) {
        (Some(existing @ Value::Object(_)), Value::Object(fields)) => {
            for (inner_key, inner_value) in fields {
                merge(existing, inner_key, inner_value);
            }
        }
        _ => {
            map.insert(key.to_string(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_engine::DISPLAY_MAX_ANGLE;
    use craft3d_math::Portable;
    use serde_json::json;

    /// The appearance is a lower-case name, System by default; a patch sets
    /// it and an unknown name is refused with nothing changed; System
    /// follows the system and is light when the system says nothing.
    #[test]
    fn the_appearance_reads_writes_and_resolves() {
        let mut state = UiState::default();
        assert_eq!(state.appearance, Appearance::System);
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["appearance"], json!("system"));
        state.apply_patch(&json!({"appearance": "dark"})).unwrap();
        assert_eq!(state.appearance, Appearance::Dark);
        let before = state.clone();
        assert!(state.apply_patch(&json!({"appearance": "dusk"})).is_err());
        assert!(state.apply_patch(&json!({"appearance": 1})).is_err());
        assert!(state.apply_patch(&json!({"appearance": "Dark"})).is_err());
        assert_eq!(state, before);
        let back: UiState = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(back.appearance, Appearance::Dark);
        // An older state without the field reads as System.
        let old: UiState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old.appearance, Appearance::System);
        use egui::Theme;
        for (appearance, system, dark) in [
            (Appearance::System, None, false),
            (Appearance::System, Some(Theme::Light), false),
            (Appearance::System, Some(Theme::Dark), true),
            (Appearance::Light, Some(Theme::Dark), false),
            (Appearance::Dark, None, true),
            (Appearance::Dark, Some(Theme::Light), true),
        ] {
            assert_eq!(
                appearance.is_dark(system),
                dark,
                "{appearance:?} {system:?}"
            );
        }
        assert_eq!(Appearance::Dark.name(), "dark");
    }

    /// The side panel's tab and view are camelCase JSON, set by a patch;
    /// an unknown tab is refused with nothing changed, and what is too
    /// long is cut.
    #[test]
    fn the_side_panel_reads_writes_and_bounds() {
        let mut state = UiState::default();
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["panels"]["sideTab"], "features");
        assert_eq!(json["sidePanel"]["checksOpen"], true);
        assert_eq!(json["sidePanel"]["showConsumed"], false);
        state
            .apply_patch(&json!({"panels": {"sideTab": "variables"}, "sidePanel": {"hover": "F3", "unfolded": ["F8"]}}))
            .unwrap();
        assert_eq!(state.panels.side_tab, SideTab::Variables);
        assert_eq!(state.side_panel.hover.as_deref(), Some("F3"));
        assert!(state.side_panel.is_open("F8"));
        // The rest of the panels as they were.
        assert!(state.panels.tree);
        let before = state.clone();
        assert!(
            state
                .apply_patch(&json!({"panels": {"sideTab": "agentx"}}))
                .is_err()
        );
        assert_eq!(state, before);
        let long = "x".repeat(MAX_FILTER_CHARS * 2);
        state.apply_patch(&json!({"sidePanel": {"filter": long, "menu": "y".repeat(MAX_SELECTION_CHARS + 1)}})).unwrap();
        assert_eq!(state.side_panel.filter.chars().count(), MAX_FILTER_CHARS);
        assert_eq!(state.side_panel.menu, None);
        state.side_panel.set_open("F8", false);
        assert!(!state.side_panel.is_open("F8"));
    }

    #[test]
    fn the_state_round_trips_through_json() {
        let state = UiState {
            camera: CameraState {
                target: [1.5, -2.0, 3.25],
                distance: 42.0,
                yaw: 135.0,
                pitch: -20.0,
                projection: ProjectionKind::Orthographic,
                fov_y: 45.0,
                auto_fit: false,
            },
            panels: Panels {
                tree: false,
                status: true,
                recovery: false,
                tree_width: 320.0,
                drawer: true,
                // The popovers' options (2026-10-31).
                look_options: false,
                analysis_options: true,
                // The side panel's tabs (2026-11-02).
                side_tab: SideTab::Bodies,
            },
            selected_feature: Some("F3".to_string()),
            notice: "Step 2: the post".to_string(),
            // The angle was added with the tessellator's angle criterion
            // (2026-10-14).
            quality: DisplayQuality {
                chordal: 1e-4,
                max_triangles: 500_000,
                angle: 20.0,
            },
            highlight: Some(FaceRef {
                feature: Some("F3".to_string()),
                face: "F3:face(2)".to_string(),
            }),
            // The fields for edges and vertices were added with them
            // (2026-10-13).
            highlight_edge: Some(EdgeRef {
                feature: Some("F3".to_string()),
                edge: None,
                between: vec!["F3:face(0)".to_string(), "F3:face(2)".to_string()],
            }),
            highlight_vertex: Some(VertexRef {
                feature: None,
                vertex: Some("F3:vertex(1)".to_string()),
                corner: Vec::new(),
            }),
            transparency: 0.5,
            // Added with the section plane (2026-10-14).
            section: SectionState {
                on: true,
                plane: Some("F3:face(2)".to_string()),
                feature: Some("F3".to_string()),
                point: None,
                normal: None,
                offset: -2.5,
                flip: true,
            },
            picked: Some(Picked {
                serial: 4,
                x: 120.5,
                y: 80.25,
                feature: Some("F3".to_string()),
                name: Some("Post".to_string()),
                face: Some("F3:face(2)".to_string()),
                face_index: Some(2),
                point: Some([1.0, 2.0, 3.5]),
                kind: Some(PickedKind::Edge),
                edge: Some(PickedEntity {
                    feature: "F3".to_string(),
                    body: "Post".to_string(),
                    name: Some("F3:edge(1)".to_string()),
                    index: 1,
                    faces: vec!["F3:face(0)".to_string(), "F3:face(2)".to_string()],
                    point: [1.0, 2.0, 3.0],
                }),
                vertex: None,
            }),
            // Added with datums in the viewport (2026-10-14).
            show: Show {
                datums: false,
                sketches: false,
                // Added with the Edges toggle (2026-10-30).
                edges: false,
            },
            // Added with the Help menu (2026-10-23).
            dialogs: Dialogs {
                about: true,
                shortcuts: false,
                // Added with the agents pill (2026-11-02).
                agents: true,
            },
            // Added with the history player (2026-10-23).
            timeline: Playback {
                playing: true,
                speed: 2.0,
                looping: true,
                orbit: false,
                motion: true,
                bounce: true,
                collisions: true,
                // Added with the light drawing while playing (2026-11-02).
                play_quality: PlayQuality::Full,
            },
            // Added with the Jigs' parameter form (2026-10-23).
            parameters: ParametersView {
                open: true,
                collapsed: vec!["Finish".to_string()],
            },
            // Added with the export dialog first (2026-10-27).
            export: ExportState {
                open: true,
                what: crate::files::ExportWhat::SheetMetal("Lid".to_string()),
                format: crate::files::ExportFormat::Dxf,
                quality: crate::files::ExportQuality::Fine,
                drawing: crate::files::DrawingPlan {
                    view: crate::files::DrawingView::Top,
                    perspective: false,
                    dashed: true,
                },
                width: "600 mm".to_string(),
                gap: String::new(),
                name: "box.dxf".to_string(),
            },
            // Added with the studio look (2026-10-29).
            look: LookState {
                mode: "studio".to_string(),
                environment: "workshop".to_string(),
                exposure: -0.5,
                rotation: 90.0,
                ground: false,
                ao: true,
                shadows: false,
                tonemap: "aces".to_string(),
                light: Some([30.0, 45.0]),
                background: "plain".to_string(),
                // Layer 2 (2026-10-30).
                reflections: false,
                contact_shadows: true,
                specular_shadow: true,
                ground_reflection: 0.5,
                bloom: 0.1,
                dof: Some(serde_json::json!({"fstop": 4.0, "focus": "picked"})),
                accumulate: Some(8),
                // The edges in the studio look (2026-10-30).
                edges: "off".to_string(),
            },
            // Added with surface analysis (2026-10-30).
            analysis: AnalysisState {
                mode: "zebra".to_string(),
                stripes: 24.0,
                direction: 30.0,
                width: 0.25,
                offset: 0.5,
                pull: [0.0, 1.0, 0.0],
                min_draft: 2.0,
                curvature: "mean".to_string(),
                range: Some([-0.5, 0.5]),
                combs: Some(serde_json::json!({"edges": true, "density": 20})),
                legend: false,
            },
            // Added with the appearance (2026-11-02).
            appearance: Appearance::Dark,
            // The side panel (2026-11-02).
            side_panel: SidePanelView {
                filter: "bore".to_string(),
                variable_filter: "hole".to_string(),
                body_filter: String::new(),
                unfolded: vec!["F3".to_string(), "datums:F13".to_string()],
                folded: vec!["Holes".to_string()],
                checks_open: false,
                show_consumed: true,
                body: Some("F23".to_string()),
                hover: Some("F3".to_string()),
                menu: None,
            },
        };
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(
            json,
            json!({
                "camera": {"target": [1.5, -2.0, 3.25], "distance": 42.0, "yaw": 135.0,
                           "pitch": -20.0, "projection": "orthographic", "fovY": 45.0,
                           "autoFit": false},
                "panels": {"tree": false, "status": true, "recovery": false, "treeWidth": 320.0,
                           "drawer": true, "lookOptions": false, "analysisOptions": true,
                           "sideTab": "bodies"},
                "sidePanel": {"filter": "bore", "variableFilter": "hole", "bodyFilter": "",
                              "unfolded": ["F3", "datums:F13"], "folded": ["Holes"],
                              "checksOpen": false, "showConsumed": true, "body": "F23",
                              "hover": "F3", "menu": null},
                "selectedFeature": "F3",
                "notice": "Step 2: the post",
                "quality": {"chordal": 1e-4, "maxTriangles": 500_000, "angle": 20.0},
                "highlight": {"feature": "F3", "face": "F3:face(2)"},
                "highlightEdge": {"feature": "F3", "edge": null,
                                  "between": ["F3:face(0)", "F3:face(2)"]},
                "highlightVertex": {"feature": null, "vertex": "F3:vertex(1)", "corner": []},
                "transparency": 0.5,
                "section": {"on": true, "plane": "F3:face(2)", "feature": "F3", "point": null,
                            "normal": null, "offset": -2.5, "flip": true},
                "picked": {"serial": 4, "x": 120.5, "y": 80.25, "feature": "F3",
                           "name": "Post", "face": "F3:face(2)", "faceIndex": 2,
                           "point": [1.0, 2.0, 3.5], "kind": "edge",
                           "edge": {"feature": "F3", "body": "Post", "name": "F3:edge(1)",
                                    "index": 1, "faces": ["F3:face(0)", "F3:face(2)"],
                                    "point": [1.0, 2.0, 3.0]},
                           "vertex": null},
                "show": {"datums": false, "sketches": false, "edges": false},
                "parameters": {"open": true, "collapsed": ["Finish"]},
                "export": {"open": true, "what": {"sheetMetal": "Lid"}, "format": "dxf",
                           "quality": "fine",
                           "drawing": {"view": "top", "perspective": false, "dashed": true},
                           "width": "600 mm", "gap": "", "name": "box.dxf"},
                "dialogs": {"about": true, "shortcuts": false, "agents": true},
                "timeline": {"playing": true, "speed": 2.0, "loop": true, "orbit": false, "motion": true, "bounce": true, "collisions": true, "playQuality": "full"},
                "look": {"mode": "studio", "environment": "workshop", "exposure": -0.5,
                         "rotation": 90.0, "ground": false, "ao": true, "shadows": false,
                         "tonemap": "aces", "light": [30.0, 45.0], "background": "plain",
                         "reflections": false, "contactShadows": true, "specularShadow": true,
                         "groundReflection": 0.5, "bloom": 0.1,
                         "dof": {"fstop": 4.0, "focus": "picked"}, "accumulate": 8,
                         "edges": "off"},
                "analysis": {"mode": "zebra", "stripes": 24.0, "direction": 30.0, "width": 0.25,
                             "offset": 0.5, "pull": [0.0, 1.0, 0.0], "minDraft": 2.0,
                             "curvature": "mean", "range": [-0.5, 0.5],
                             "combs": {"edges": true, "density": 20}, "legend": false},
                "appearance": "dark",
            })
        );
        let back: UiState = serde_json::from_value(json).unwrap();
        assert_eq!(back, state);
        assert_eq!(back.clone().validated().unwrap(), state, "already in range");
        // The default state round-trips too, and an empty object is the
        // default.
        let default = UiState::default();
        let text = serde_json::to_string(&default).unwrap();
        assert_eq!(serde_json::from_str::<UiState>(&text).unwrap(), default);
        assert_eq!(serde_json::from_str::<UiState>("{}").unwrap(), default);
    }

    /// A tab's view is the document's part of the state, and putting it
    /// back leaves the window's part alone.
    #[test]
    fn a_tab_view_swaps_the_documents_part_of_the_state_only() {
        let mut state = UiState::default();
        state
            .apply_patch(
                &json!({"camera": {"yaw": 10, "distance": 5, "projection": "orthographic"},
                                 "section": {"on": true}, "transparency": 0.4,
                                 "selectedFeature": "F2", "notice": "keep",
                                 "panels": {"tree": false}, "show": {"datums": false},
                                 "quality": {"chordal": 1e-4},
                                 "highlight": {"face": "F1:face(0)"}}),
            )
            .unwrap();
        let first = TabView::of(&state);
        let window = (
            state.panels,
            state.notice.clone(),
            state.quality,
            state.show,
        );
        // Another document starts as a fresh one.
        TabView::default().put(&mut state);
        assert_eq!(state.camera, CameraState::default());
        assert_eq!(state.camera.projection, ProjectionKind::Perspective);
        assert!(state.camera.auto_fit);
        assert_eq!(state.section, SectionState::default());
        assert_eq!(state.transparency, 0.0);
        assert_eq!(state.selected_feature, None);
        assert_eq!(state.highlight, None);
        assert_eq!(
            (
                state.panels,
                state.notice.clone(),
                state.quality,
                state.show
            ),
            window
        );
        // And the first comes back as it was.
        first.clone().put(&mut state);
        assert_eq!(TabView::of(&state), first);
        assert_eq!(state.camera.yaw, 10.0);
        assert!(state.section.on);
    }

    #[test]
    fn a_patch_changes_only_what_it_names() {
        let mut state = UiState::default();
        state
            .apply_patch(&json!({"camera": {"yaw": 10, "distance": 250.5}}))
            .unwrap();
        assert_eq!((state.camera.yaw, state.camera.distance), (10.0, 250.5));
        assert_eq!(state.camera.pitch, CameraState::default().pitch);
        assert_eq!(state.camera.fov_y, 30.0);
        assert_eq!(state.panels, Panels::default());
        // Placing the camera by hand stops the automatic refit, unless the
        // same request asks to keep it.
        assert!(!state.camera.auto_fit);
        state
            .apply_patch(&json!({"camera": {"target": [1, 2, 3], "autoFit": true}}))
            .unwrap();
        assert_eq!(state.camera.target, [1.0, 2.0, 3.0]);
        assert!(state.camera.auto_fit);
        // Turning the camera does not.
        state.apply_patch(&json!({"camera": {"pitch": 5}})).unwrap();
        assert!(state.camera.auto_fit);

        state
            .apply_patch(&json!({"panels": {"tree": false}, "notice": "hello",
                                 "selectedFeature": "F2"}))
            .unwrap();
        assert!(!state.panels.tree && state.panels.status);
        assert_eq!(state.notice, "hello");
        assert_eq!(state.selected_feature.as_deref(), Some("F2"));
        state
            .apply_patch(&json!({"selectedFeature": null}))
            .unwrap();
        assert_eq!(state.selected_feature, None);
        // An empty patch is no change.
        let before = state.clone();
        state.apply_patch(&json!({})).unwrap();
        assert_eq!(state, before);
    }

    #[test]
    fn named_views_are_the_directions_view_render_uses() {
        // (view, the direction towards the eye in view.render)
        let directions = [
            ("iso", [1.0, -1.0, 0.8]),
            ("front", [0.0, -1.0, 0.0]),
            ("back", [0.0, 1.0, 0.0]),
            ("right", [1.0, 0.0, 0.0]),
            ("left", [-1.0, 0.0, 0.0]),
            ("top", [0.0, 0.0, 1.0]),
            ("bottom", [0.0, 0.0, -1.0]),
        ];
        for (view, toward) in directions {
            let mut state = UiState::default();
            state.apply_patch(&json!({"view": view})).unwrap();
            let (yaw, pitch) = (
                state.camera.yaw.to_radians(),
                state.camera.pitch.to_radians(),
            );
            let d = [
                pitch.pcos() * yaw.pcos(),
                pitch.pcos() * yaw.psin(),
                pitch.psin(),
            ];
            let length = toward.iter().map(|c| c * c).sum::<f64>().sqrt();
            for (got, want) in d.iter().zip(toward) {
                assert!((got - want / length).abs() < 1e-12, "{view}: {d:?}");
            }
        }
        let mut state = UiState::default();
        let error = state.apply_patch(&json!({"view": "sideways"})).unwrap_err();
        assert!(
            error.contains("sideways") && error.contains("iso"),
            "{error}"
        );
    }

    #[test]
    fn garbage_is_refused_and_changes_nothing() {
        let original = UiState {
            notice: "keep me".to_string(),
            ..UiState::default()
        };
        let garbage = [
            json!(null),
            json!(7),
            json!("camera"),
            json!([1, 2, 3]),
            json!({"camrea": {}}),
            json!({"camera": 5}),
            json!({"camera": "iso"}),
            json!({"camera": [1, 2, 3]}),
            json!({"camera": {"yaw": "ninety"}}),
            json!({"camera": {"yaw": null}}),
            json!({"camera": {"yaw": true}}),
            json!({"camera": {"yaw": [90]}}),
            json!({"camera": {"zoom": 2}}),
            json!({"camera": {"target": [1, 2]}}),
            json!({"camera": {"target": [1, 2, 3, 4]}}),
            json!({"camera": {"target": "origin"}}),
            json!({"camera": {"target": [1, "2", 3]}}),
            json!({"camera": {"target": {"x": 1}}}),
            json!({"camera": {"projection": "isometric"}}),
            json!({"camera": {"projection": 1}}),
            json!({"camera": {"autoFit": "yes"}}),
            json!({"panels": {"tree": 1}}),
            json!({"panels": {"inspector": true}}),
            json!({"panels": true}),
            json!({"notice": 5}),
            json!({"notice": ["a"]}),
            json!({"selectedFeature": 3}),
            json!({"selectedFeature": {"id": "F1"}}),
            json!({"quality": 1e-4}),
            json!({"quality": "fine"}),
            json!({"quality": {"chordal": "fine"}}),
            json!({"quality": {"chordal": null}}),
            json!({"quality": {"chordal": [1e-4]}}),
            json!({"quality": {"maxTriangles": -1}}),
            json!({"quality": {"maxTriangles": 1.5}}),
            json!({"quality": {"maxTriangles": "many"}}),
            json!({"quality": {"level": "high"}}),
            json!({"quality": {"chordal": 1e-4, "tolerance": 0.01}}),
            json!({"view": 3}),
            json!({"view": "upside-down"}),
            json!({"view": null}),
            // One good field does not let a bad one through.
            json!({"notice": "changed", "camera": {"yaw": "x"}}),
            json!({"camera": {"yaw": 1}, "bogus": 1}),
            // Nor does a good view: the camera is not turned either.
            json!({"view": "top", "panels": {"tree": 3}}),
            json!({"view": "front", "bogus": 1}),
        ];
        for patch in &garbage {
            let mut state = original.clone();
            let error = state.apply_patch(patch).unwrap_err();
            assert!(!error.is_empty(), "{patch}");
            assert_eq!(state, original, "{patch} changed the state");
        }
    }

    #[test]
    fn huge_numbers_are_clamped_and_angles_wrapped() {
        let mut state = UiState::default();
        state
            .apply_patch(&json!({"camera": {
                "target": [1e300, -1e300, 5],
                "distance": 1e308,
                "yaw": 725,
                "pitch": 1e9,
                "fovY": 1e9,
            }}))
            .unwrap();
        assert_eq!(state.camera.target, [MAX_EXTENT, -MAX_EXTENT, 5.0]);
        assert_eq!(state.camera.distance, MAX_EXTENT);
        assert_eq!(state.camera.yaw, 5.0);
        assert_eq!(state.camera.pitch, 90.0);
        assert_eq!(state.camera.fov_y, MAX_FOV);
        state
            .apply_patch(
                &json!({"camera": {"distance": -3, "yaw": -190, "pitch": -1e9, "fovY": 0}}),
            )
            .unwrap();
        assert_eq!(state.camera.distance, MIN_DISTANCE);
        assert_eq!(state.camera.yaw, 170.0);
        assert_eq!(state.camera.pitch, -90.0);
        assert_eq!(state.camera.fov_y, MIN_FOV);
        state.apply_patch(&json!({"camera": {"yaw": 180}})).unwrap();
        assert_eq!(state.camera.yaw, 180.0);
        state
            .apply_patch(&json!({"camera": {"yaw": -180}}))
            .unwrap();
        assert_eq!(state.camera.yaw, 180.0);

        // Texts are cut, not refused, and stay one line.
        let long = "é".repeat(5000);
        state
            .apply_patch(&json!({"notice": long, "selectedFeature": "F".repeat(5000)}))
            .unwrap();
        assert_eq!(state.notice.chars().count(), MAX_NOTICE_CHARS);
        assert_eq!(
            state.selected_feature.as_ref().unwrap().len(),
            MAX_SELECTION_CHARS
        );
        state
            .apply_patch(&json!({"notice": "two\nlines\tand a tab", "selectedFeature": ""}))
            .unwrap();
        assert_eq!(state.notice, "two lines and a tab");
        assert_eq!(state.selected_feature, None);
    }

    /// The display quality is state like the camera: patched field by
    /// field, clamped, and the engine's defaults when nothing is set.
    #[test]
    fn the_display_quality_is_patched_and_clamped() {
        let mut state = UiState::default();
        assert_eq!(
            state.quality,
            DisplayQuality {
                chordal: DISPLAY_RELATIVE_TOLERANCE,
                max_triangles: DISPLAY_MAX_TRIANGLES as u64,
                angle: DISPLAY_MAX_ANGLE_DEGREES,
            }
        );
        assert_eq!(state.quality.scene_quality(), SceneQuality::display());
        state
            .apply_patch(&json!({"quality": {"chordal": 1e-4}}))
            .unwrap();
        assert_eq!(state.quality.chordal, 1e-4);
        assert_eq!(state.quality.max_triangles, DISPLAY_MAX_TRIANGLES as u64);
        state
            .apply_patch(&json!({"quality": {"maxTriangles": 250_000}}))
            .unwrap();
        assert_eq!(state.quality.chordal, 1e-4);
        assert_eq!(state.quality.max_triangles, 250_000);
        assert_eq!(
            state.quality.scene_quality(),
            SceneQuality::Display {
                relative: 1e-4,
                max_triangles: 250_000,
                max_angle: Some(DISPLAY_MAX_ANGLE),
            }
        );
        // The angle in degrees; none at 0, clamped above.
        state
            .apply_patch(&json!({"quality": {"angle": 0}}))
            .unwrap();
        assert!(matches!(
            state.quality.scene_quality(),
            SceneQuality::Display {
                max_angle: None,
                ..
            }
        ));
        state
            .apply_patch(&json!({"quality": {"angle": 500}}))
            .unwrap();
        assert_eq!(state.quality.angle, MAX_ANGLE_DEGREES);
        state
            .apply_patch(&json!({"quality": {"angle": -3}}))
            .unwrap();
        assert_eq!(state.quality.angle, 0.0);
        state
            .apply_patch(&json!({"quality": {"angle": 10}}))
            .unwrap();
        assert_eq!(state.quality.angle, 10.0);
        // Out of range is clamped, not refused.
        state
            .apply_patch(&json!({"quality": {"chordal": 1e-30, "maxTriangles": 1}}))
            .unwrap();
        assert_eq!(
            (state.quality.chordal, state.quality.max_triangles),
            (MIN_CHORDAL, MIN_TRIANGLES)
        );
        state
            .apply_patch(&json!({"quality": {"chordal": 5.0, "maxTriangles": u64::MAX}}))
            .unwrap();
        assert_eq!(
            (state.quality.chordal, state.quality.max_triangles),
            (MAX_CHORDAL, MAX_TRIANGLES)
        );
        // A quality nobody can mean is an error.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let quality = DisplayQuality {
                chordal: bad,
                ..DisplayQuality::default()
            };
            let error = quality.validated().unwrap_err();
            assert!(error.contains("quality.chordal"), "{error}");
            let state = UiState {
                quality,
                ..UiState::default()
            };
            assert!(state.validated().is_err());
        }
    }

    /// JSON cannot carry NaN, but Rust code can hand one over.
    #[test]
    fn numbers_that_are_not_finite_are_an_error() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let fields: [fn(&mut CameraState, f64); 5] = [
                |c, v| c.target[1] = v,
                |c, v| c.distance = v,
                |c, v| c.yaw = v,
                |c, v| c.pitch = v,
                |c, v| c.fov_y = v,
            ];
            for set in fields {
                let mut camera = CameraState::default();
                set(&mut camera, bad);
                let error = camera.validated().unwrap_err();
                assert!(error.contains("finite"), "{error}");
                let state = UiState {
                    camera,
                    ..UiState::default()
                };
                assert!(state.validated().is_err());
            }
        }
        // Text that looks like a number is still text.
        let mut state = UiState::default();
        for text in ["NaN", "Infinity", "1e999"] {
            assert!(
                state
                    .apply_patch(&json!({"camera": {"distance": text}}))
                    .is_err()
            );
        }
        // And a number too large for f64 never becomes a request at all.
        assert!(serde_json::from_str::<Value>(r#"{"camera": {"distance": 1e999}}"#).is_err());
    }

    #[test]
    fn a_highlight_is_set_whole_and_a_pick_cannot_be_set() {
        let mut state = UiState::default();
        assert_eq!(
            serde_json::to_value(&state).unwrap()["highlight"],
            json!(null)
        );
        assert_eq!(serde_json::to_value(&state).unwrap()["picked"], json!(null));
        state
            .apply_patch(&json!({"highlight": {"feature": "Box", "face": "F1:face(1)"}}))
            .unwrap();
        assert_eq!(
            state.highlight,
            Some(FaceRef {
                feature: Some("Box".to_string()),
                face: "F1:face(1)".to_string()
            })
        );
        // Another face without a feature is not the old feature's.
        state
            .apply_patch(&json!({"highlight": {"face": "F2:face(0)"}}))
            .unwrap();
        assert_eq!(
            state.highlight,
            Some(FaceRef {
                feature: None,
                face: "F2:face(0)".to_string()
            })
        );
        // Cleared with null; a face without a name is no highlight.
        state.apply_patch(&json!({"highlight": null})).unwrap();
        assert_eq!(state.highlight, None);
        state
            .apply_patch(&json!({"highlight": {"feature": "", "face": ""}}))
            .unwrap();
        assert_eq!(state.highlight, None);
        // Texts are cut to their limits.
        state
            .apply_patch(
                &json!({"highlight": {"feature": "f".repeat(9000), "face": "n".repeat(9000)}}),
            )
            .unwrap();
        let kept = state.highlight.clone().unwrap();
        assert_eq!(kept.feature.unwrap().chars().count(), MAX_SELECTION_CHARS);
        assert_eq!(kept.face.chars().count(), MAX_FACE_NAME_CHARS);
        // Wrong shapes are refused and change nothing.
        let before = state.clone();
        for bad in [
            json!({"highlight": "F1:face(1)"}),
            json!({"highlight": 3}),
            json!({"highlight": {"face": 3}}),
            json!({"highlight": {"face": "F1:face(1)", "body": "Box"}}),
            json!({"highlight": {"feature": 1, "face": "F1:face(1)"}}),
            json!({"picked": null}),
            json!({"picked": {"x": 1, "y": 2}}),
            json!({"notice": "x", "picked": {"serial": 9}}),
        ] {
            let error = state.apply_patch(&bad).unwrap_err();
            assert!(!error.is_empty(), "{bad}");
            assert_eq!(state, before, "{bad}");
        }
        assert!(
            state
                .apply_patch(&json!({"picked": null}))
                .unwrap_err()
                .contains("cannot be set")
        );
    }
}
