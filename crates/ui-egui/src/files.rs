//! What the hosts' file actions share, whether the file is on disk (the
//! desktop's dialogs) or in the session's memory (the browser): the
//! names they propose, and what `file.export` is given for the window's
//! selection. Hosts run the commands; nothing here touches a file.

use craft3d_engine::{CommandError, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

/// The extension of the native format.
pub const EXTENSION: &str = "3dcraft";

/// The extension of a published Jig: the same format, opened to be used.
pub const JIG_EXTENSION: &str = "3dcraftjig";

/// The extension of an assembly.
pub const ASSEMBLY_EXTENSION: &str = "3dcraftasm";

/// The extensions Open reads: a document, a published Jig and an
/// assembly.
pub const OPEN_EXTENSIONS: [&str; 3] = [EXTENSION, JIG_EXTENSION, ASSEMBLY_EXTENSION];

/// The extensions `file.import` reads (STEP, FreeCAD documents and BRep
/// files), in the cases dialogs that filter by case meet them in.
pub const IMPORT_EXTENSIONS: [&str; 12] = [
    "step", "stp", "STEP", "STP", "FCStd", "fcstd", "FCSTD", "brep", "brp", "BREP", "BRP", "Fcstd",
];

/// The formats `file.export` writes, as the export dialog offers them: a
/// name and its extensions. The format follows the extension of the file
/// the person names.
pub const EXPORT_FORMATS: [(&str, &[&str]); 6] = [
    ("STEP", &["step", "stp"]),
    ("STL", &["stl"]),
    ("3MF", &["3mf"]),
    ("OBJ", &["obj"]),
    ("DXF", &["dxf"]),
    ("SVG", &["svg"]),
];

/// A format `file.export` writes, as the export dialog offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum ExportFormat {
    #[default]
    #[serde(rename = "step")]
    Step,
    #[serde(rename = "stl")]
    Stl,
    #[serde(rename = "3mf")]
    ThreeMf,
    #[serde(rename = "obj")]
    Obj,
    #[serde(rename = "dxf")]
    Dxf,
    #[serde(rename = "svg")]
    Svg,
}

impl ExportFormat {
    /// Every format, in the order the dialog offers them.
    pub const ALL: [ExportFormat; 6] = [
        Self::Step,
        Self::Stl,
        Self::ThreeMf,
        Self::Obj,
        Self::Dxf,
        Self::Svg,
    ];

    /// Its place in [`EXPORT_FORMATS`].
    fn index(self) -> usize {
        match self {
            Self::Step => 0,
            Self::Stl => 1,
            Self::ThreeMf => 2,
            Self::Obj => 3,
            Self::Dxf => 4,
            Self::Svg => 5,
        }
    }

    /// What the dialog calls it: "STEP".
    pub fn label(self) -> &'static str {
        EXPORT_FORMATS
            .get(self.index())
            .map_or("", |(name, _)| name)
    }

    /// The extensions a file of this format may have, the one a new name
    /// gets first.
    pub fn extensions(self) -> &'static [&'static str] {
        EXPORT_FORMATS
            .get(self.index())
            .map_or(&[], |(_, extensions)| extensions)
    }

    /// The extension a new name gets: `stl`.
    pub fn extension(self) -> &'static str {
        self.extensions().first().copied().unwrap_or_default()
    }

    /// The format of a file with this extension (any case).
    pub fn from_extension(extension: &str) -> Option<Self> {
        let extension = extension.to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|f| f.extensions().contains(&extension.as_str()))
    }

    /// The format a file of this name is written in, by its extension.
    pub fn of_name(name: &str) -> Option<Self> {
        Self::from_extension(&extension_of(name)?)
    }

    /// What a file of this format holds.
    pub fn kind(self) -> ExportKind {
        match self {
            Self::Step | Self::Stl | Self::ThreeMf | Self::Obj => ExportKind::Bodies,
            Self::Dxf | Self::Svg => ExportKind::Outline,
        }
    }

    /// A mesh: its quality applies.
    pub fn is_mesh(self) -> bool {
        matches!(self, Self::Stl | Self::ThreeMf | Self::Obj)
    }

    /// The formats of a kind, in the dialog's order.
    pub fn of_kind(kind: ExportKind) -> Vec<ExportFormat> {
        Self::ALL.into_iter().filter(|f| f.kind() == kind).collect()
    }
}

/// `name` as a file of `format`: as it is when its extension is one of
/// the format's (in any case); with another export format's extension,
/// or the native one, replaced; and with the format's extension appended
/// otherwise (`bracket.v2` → `bracket.v2.stl`). So a file is never named
/// for another format than the bytes it holds. An empty name is
/// `untitled`.
pub fn export_file_name(name: &str, format: ExportFormat) -> String {
    let trimmed = name.trim().trim_end_matches('.');
    let name = if trimmed.is_empty() {
        "untitled"
    } else {
        trimmed
    };
    let wanted = format.extension();
    match extension_of(name) {
        Some(extension) if format.extensions().contains(&extension.as_str()) => name.to_string(),
        Some(extension)
            if ExportFormat::from_extension(&extension).is_some()
                || extension == EXTENSION
                || extension == JIG_EXTENSION =>
        {
            // The extension's bytes are the name's last ones (lower case
            // keeps their length).
            let stem = name
                .len()
                .checked_sub(extension.len().saturating_add(1))
                .and_then(|end| name.get(..end))
                .filter(|stem| !stem.is_empty())
                .unwrap_or("untitled");
            format!("{stem}.{wanted}")
        }
        _ => format!("{name}.{wanted}"),
    }
}

/// The name a dialog proposes for a document saved for the first time or
/// under another name: a Jig in use (`x.3dcraftjig`) is saved as a model,
/// `x.3dcraft`.
pub fn suggested_name(path: Option<&str>) -> String {
    let name = path
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| format!("untitled.{EXTENSION}"), str::to_string);
    if extension_of(&name).as_deref() == Some(JIG_EXTENSION) {
        let stem = Path::new(&name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("untitled");
        format!("{stem}.{EXTENSION}")
    } else {
        name
    }
}

/// The name Export as Jig proposes: the document's file's, as a
/// `.3dcraftjig`.
pub fn suggested_jig_name(path: Option<&str>) -> String {
    let stem = path
        .and_then(|path| Path::new(path).file_stem())
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("untitled");
    format!("{stem}.{JIG_EXTENSION}")
}

/// The name the export dialog proposes: the document's file's, with the
/// extension of the last export (STEP the first time).
pub fn suggested_export_name(path: Option<&str>, extension: Option<&str>) -> String {
    let stem = path
        .and_then(|path| Path::new(path).file_stem())
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("untitled");
    format!("{stem}.{}", extension.unwrap_or("step"))
}

/// What an export to a file of this name holds, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    /// Bodies: every body, or those selected (STEP, STL, 3MF, OBJ).
    Bodies,
    /// The flat outline of faces or of a sketch, or with nothing selected
    /// the model as seen (DXF, SVG).
    Outline,
}

/// A file's name without its folder and its 3DCraft extension, as the
/// status bar names a file being opened (`welding-table`).
pub fn file_stem(name: &str) -> Option<String> {
    let name = name.rsplit(['/', '\\']).next()?;
    let stem = name
        .strip_suffix(".3dcraftjig")
        .or_else(|| name.strip_suffix(".3dcraft"))
        .unwrap_or(name);
    (!stem.is_empty()).then(|| stem.to_string())
}

/// The lower-case extension of a file name, if it has one.
pub fn extension_of(name: &str) -> Option<String> {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// What an export to the file `name` holds; `None` for a name whose
/// extension `file.export` does not write.
pub fn export_kind(name: &str) -> Option<ExportKind> {
    match extension_of(name)?.as_str() {
        "step" | "stp" | "stl" | "3mf" | "obj" => Some(ExportKind::Bodies),
        "dxf" | "svg" => Some(ExportKind::Outline),
        _ => None,
    }
}

/// A field of a command's result; null when it is not there.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}

/// Runs a command, a panic turned into an error.
pub fn guarded(session: &mut Session, command: &str, params: &Value) -> Result<Value, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.execute(command, params)
    }))
    .unwrap_or_else(|_| {
        Err(CommandError::new(
            "internal error (a bug in 3DCraft, please report it); the document was kept",
        ))
    })
    .map_err(|e| e.message().to_string())
}

/// The parameter `file.export` is given for the window's selection, if
/// any, to write the file `name`: `None` exports every body. An error,
/// saying why, when `file.export` cannot write what is selected: never
/// something else in its place.
///
/// - Nothing selected: a STEP or mesh file holds every body; a DXF or SVG
///   file is a drawing of the model as the viewport shows it, to scale
///   (`file.export` with `view: "current"`, the camera the window gave
///   the session; hidden lines left out).
/// - Bodies only, to STEP or a mesh file: those bodies (`bodies`, by the
///   ids of their features); every body of the document is as nothing
///   selected.
/// - Faces only, to DXF or SVG: those faces on one sheet (`faces`; a face
///   that is not flat is refused by `file.export`, naming it).
/// - One sketch only, to DXF or SVG: its outline (`sketch`).
/// - The body of one sheet-metal part or fold only, to DXF or SVG: its
///   flat pattern (`sheetMetal`) or crease pattern (`fold`).
/// - Anything else is refused: `file.export` writes bodies to STEP and
///   the mesh formats, and faces or one sketch to DXF and SVG.
pub fn export_selection(
    session: &mut Session,
    name: &str,
) -> Result<Option<(&'static str, Value)>, String> {
    const ID: &str = "file.export";
    let Some(kind) = export_kind(name) else {
        // `file.export` says which names it knows.
        return Ok(None);
    };
    let extension = extension_of(name).unwrap_or_default();
    let selection = guarded(session, "select.get", &json!({ "context": false }))
        .map_err(|e| format!("{ID}: {e}"))?;
    let items: &[Value] = field(&selection, "items")
        .as_array()
        .map_or(&[], Vec::as_slice);
    let summary = field(&selection, "summary")
        .as_str()
        .unwrap_or("")
        .to_string();
    let of_kind = |kind: &str| items.iter().filter(|i| field(i, "kind") == kind).count();
    let (faces, bodies, sketches) = (of_kind("face"), of_kind("body"), of_kind("sketch"));
    match kind {
        ExportKind::Bodies => {
            if items.is_empty() {
                return Ok(None);
            }
            if bodies == items.len() {
                let all = guarded(session, "select.query", &json!({ "kind": "body" }))
                    .map_err(|e| format!("{ID}: {e}"))?;
                if field(&all, "count").as_u64() == Some(bodies as u64) {
                    return Ok(None);
                }
                // By the number of the feature's id: `file.export` reads a
                // string as a name first, and an old file may have a
                // feature named like another's id.
                let list: Vec<Value> = items
                    .iter()
                    .map(|i| {
                        match field(i, "feature")
                            .as_str()
                            .and_then(|id| id.strip_prefix('F'))
                            .and_then(|digits| digits.parse::<u64>().ok())
                        {
                            Some(n) => Value::from(n),
                            None => field(i, "name").clone(),
                        }
                    })
                    .collect();
                return Ok(Some(("bodies", Value::Array(list))));
            }
            Err(format!(
                "{ID}: a .{extension} file holds bodies; it cannot hold what is selected ({summary}). Select only bodies to export those, clear the selection to export every body, or export flat faces or a sketch as .dxf or .svg"
            ))
        }
        ExportKind::Outline => {
            if items.is_empty() {
                // The model as seen from the viewport (`view_export.rs` in
                // the engine; without `view` it would be the iso view).
                return Ok(Some(("view", Value::String("current".to_string()))));
            }
            if faces == items.len() {
                let list: Vec<Value> = items
                    .iter()
                    .map(|i| json!({ "face": field(i, "name"), "feature": field(i, "feature") }))
                    .collect();
                return Ok(Some(("faces", Value::Array(list))));
            }
            if bodies == 1 && items.len() == 1 {
                let item = items.first().unwrap_or(&Value::Null);
                let pattern = flat_patterns(session)
                    .into_iter()
                    .find(|p| field(item, "feature").as_str() == Some(p.id.as_str()));
                if let Some(pattern) = pattern {
                    return Ok(Some((pattern.kind.param(), Value::String(pattern.feature))));
                }
            }
            if sketches == 1 && items.len() == 1 {
                let item = items.first().unwrap_or(&Value::Null);
                let id = match field(item, "feature").as_str() {
                    Some(id) if !id.is_empty() => id,
                    _ => field(item, "name").as_str().unwrap_or_default(),
                };
                return Ok(Some(("sketch", Value::String(id.to_string()))));
            }
            Err(format!(
                "{ID}: a .{extension} file is the outline of flat faces or of one sketch, or the flat pattern of one sheet-metal part or fold, and the selection is {summary}; select only flat faces, one sketch, or the body of one sheet-metal part or fold"
            ))
        }
    }
}

/// How finely a mesh export follows the model, as the export dialog
/// offers it (`file.export {quality}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportQuality {
    Coarse,
    #[default]
    Normal,
    Fine,
}

impl ExportQuality {
    /// Every quality, coarsest first.
    pub const ALL: [ExportQuality; 3] = [Self::Coarse, Self::Normal, Self::Fine];

    /// What `file.export` calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Coarse => "coarse",
            Self::Normal => "normal",
            Self::Fine => "fine",
        }
    }

    /// What the dialog calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse",
            Self::Normal => "Normal",
            Self::Fine => "Fine",
        }
    }

    /// What the dialog says of it under the pointer.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Coarse => {
                "Fewer triangles: a small file, curves in visible steps (20° per chord)"
            }
            Self::Normal => "Round to the eye and to a printer (10° per chord); the default",
            Self::Fine => "Smooth up close: four times finer, 5° per chord, a larger file",
        }
    }
}

/// The view a drawing of the model as seen is made from (DXF or SVG of
/// the model; `file.export {view}`), as the export dialog offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DrawingView {
    /// The viewport's own camera.
    #[default]
    Current,
    Iso,
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
}

impl DrawingView {
    /// Every view, the viewport's first.
    pub const ALL: [DrawingView; 8] = [
        Self::Current,
        Self::Iso,
        Self::Front,
        Self::Back,
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
    ];

    /// What `file.export` calls it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Iso => "iso",
            Self::Front => "front",
            Self::Back => "back",
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    /// What the dialog calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "As in the viewport",
            Self::Iso => "Iso",
            Self::Front => "Front",
            Self::Back => "Back",
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        }
    }
}

/// How a drawing of the model as seen is made: the view, perspective or
/// to scale, and the hidden lines dashed or left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct DrawingPlan {
    pub view: DrawingView,
    /// Perspective (an illustration) instead of orthographic (to scale).
    pub perspective: bool,
    /// The hidden lines drawn dashed instead of left out.
    pub dashed: bool,
}

/// What goes into an exported file, as the export dialog's *What* offers
/// it. In JSON (`ui.set {"export": {"what": …}}`): `"model"`,
/// `"selection"`, `{"set": name}`, `{"sheetMetal": part}`, `{"fold":
/// fold}` or `"everyPart"`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportWhat {
    /// The whole model: every body (STEP, a mesh), or a drawing of it as
    /// seen (DXF, SVG; `file.export {view, projection, hidden}`).
    #[default]
    Model,
    /// What is selected (`select.get`), as far as the format holds it
    /// ([`export_selection`]).
    Selection,
    /// A sheet of a named set's flat faces (`file.export {set, width,
    /// gap}`; DXF, SVG).
    Set(String),
    /// A sheet-metal part's flat pattern (`file.export {sheetMetal}`).
    SheetMetal(String),
    /// A fold's crease pattern (`file.export {fold}`).
    Fold(String),
    /// Every sheet-metal part's flat pattern on one sheet (`file.export
    /// {sheetMetal: [..], width, gap}`).
    EveryPart,
}

impl ExportWhat {
    /// Only DXF and SVG hold it (a sheet or a pattern).
    pub fn is_outline_only(&self) -> bool {
        !matches!(self, Self::Model | Self::Selection)
    }

    /// It is laid out on a sheet with a width and a gap.
    pub fn has_sheet(&self) -> bool {
        matches!(self, Self::Set(_) | Self::EveryPart)
    }
}

/// What a pattern of a DXF or SVG export is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatternKind {
    /// A sheet-metal part: its flat pattern, with its bend lines.
    SheetMetal,
    /// A fold: its crease pattern.
    Fold,
    /// Every sheet-metal part: their flat patterns side by side on one
    /// sheet, with its width and gap (offered for two parts or more).
    EveryPart,
}

impl PatternKind {
    /// Both, in the order the dialog offers them.
    pub const ALL: [PatternKind; 2] = [Self::SheetMetal, Self::Fold];

    /// The parameter of `file.export` that draws it, which is also the
    /// feature's type (`select.query`'s `type`).
    pub fn param(self) -> &'static str {
        match self {
            Self::SheetMetal | Self::EveryPart => "sheetMetal",
            Self::Fold => "fold",
        }
    }

    /// What the dialog calls it, before the feature's name.
    pub fn label(self) -> &'static str {
        match self {
            Self::SheetMetal => "Flat pattern of",
            Self::Fold => "Crease pattern of",
            Self::EveryPart => "Flat patterns of",
        }
    }
}

/// The flat pattern of one sheet-metal part, or the crease pattern of one
/// fold, as the export dialog offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlatPattern {
    pub kind: PatternKind,
    /// The feature's name, as `file.export` is given it ("every part"
    /// for [`PatternKind::EveryPart`]).
    pub feature: String,
    /// The feature's id (`F3`), as the selection names its body (empty
    /// for [`PatternKind::EveryPart`]).
    pub id: String,
}

impl FlatPattern {
    /// What the dialog shows: "Flat pattern of Part".
    pub fn label(&self) -> String {
        format!("{} {}", self.kind.label(), self.feature)
    }

    /// The export's *What* that draws it.
    pub fn what(&self) -> ExportWhat {
        match self.kind {
            PatternKind::SheetMetal => ExportWhat::SheetMetal(self.feature.clone()),
            PatternKind::Fold => ExportWhat::Fold(self.feature.clone()),
            PatternKind::EveryPart => ExportWhat::EveryPart,
        }
    }
}

/// The sheet-metal parts and folds of the active document, in history
/// order (the parts first), and with two parts or more every part on one
/// sheet: each a pattern a DXF or SVG export can draw, of the features
/// that were built (not switched off, rolled back or failed). None when
/// there is no document or the query fails.
pub fn flat_patterns(session: &mut Session) -> Vec<FlatPattern> {
    let mut out = Vec::new();
    for kind in PatternKind::ALL {
        let Ok(found) = guarded(
            session,
            "select.query",
            // Only what was built: a part switched off, rolled back or
            // failed has no pattern to draw.
            &json!({"kind": "feature", "where": {"type": kind.param(), "status": "ok"}, "limit": 256}),
        ) else {
            continue;
        };
        let items = field(&found, "items")
            .as_array()
            .map_or(&[][..], Vec::as_slice);
        for item in items {
            if let (Some(feature), Some(id)) = (
                field(item, "name").as_str(),
                field(item, "feature").as_str(),
            ) {
                out.push(FlatPattern {
                    kind,
                    feature: feature.to_string(),
                    id: id.to_string(),
                });
            }
        }
    }
    let parts = out
        .iter()
        .filter(|p| p.kind == PatternKind::SheetMetal)
        .count();
    if parts > 1 {
        out.push(FlatPattern {
            kind: PatternKind::EveryPart,
            feature: "every part".to_string(),
            id: String::new(),
        });
    }
    out
}

/// Longest width, gap or file name the export dialog keeps, in
/// characters.
pub const MAX_EXPORT_TEXT: usize = 200;

/// How the person chose to export, in the export dialog: what goes into
/// the file and how (the format is the file's name's).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExportPlan {
    pub what: ExportWhat,
    /// How finely a mesh follows the model (STL, 3MF, OBJ).
    pub quality: ExportQuality,
    /// A drawing of the model as seen (the model to DXF or SVG).
    pub drawing: DrawingPlan,
    /// The sheet's width as typed (`600 mm`, `600` in the document's
    /// unit), for a set's sheet and every part's; empty for no limit.
    pub width: String,
    /// The gap between two outlines as typed; empty for 5 mm.
    pub gap: String,
}

/// Whether a file of this name is a mesh (`quality` applies).
pub fn is_mesh(name: &str) -> bool {
    ExportFormat::of_name(name).is_some_and(ExportFormat::is_mesh)
}

/// What is selected, as the export dialog offers it: in which formats it
/// can be exported, and why in none when it cannot.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SelectionChoice {
    /// `select.get`'s summary: "1 body, 2 faces".
    pub summary: String,
    /// The formats that hold it ([`export_selection`]).
    pub formats: Vec<ExportFormat>,
    /// Why no format holds it, for the person; `None` when one does.
    pub problem: Option<String>,
}

/// What the export dialog offers for the active document, read when it
/// opens: the selection, the named sets that may hold faces, and the
/// sheet-metal parts' and folds' patterns.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExportChoices {
    /// `None` when nothing is selected.
    pub selection: Option<SelectionChoice>,
    pub sets: Vec<String>,
    pub patterns: Vec<FlatPattern>,
}

impl ExportChoices {
    /// Every *What* the dialog lists, in its order: the model, the
    /// selection (when something is selected, even when no format holds
    /// it: the dialog says why), the sets' sheets, the patterns.
    pub fn whats(&self) -> Vec<ExportWhat> {
        let mut out = vec![ExportWhat::Model];
        if self.selection.is_some() {
            out.push(ExportWhat::Selection);
        }
        out.extend(self.sets.iter().cloned().map(ExportWhat::Set));
        out.extend(self.patterns.iter().map(FlatPattern::what));
        out
    }

    /// The formats that hold `what`, in the dialog's order: none for what
    /// this document does not have.
    pub fn formats(&self, what: &ExportWhat) -> Vec<ExportFormat> {
        match what {
            ExportWhat::Model => ExportFormat::ALL.to_vec(),
            ExportWhat::Selection => self
                .selection
                .as_ref()
                .map(|s| s.formats.clone())
                .unwrap_or_default(),
            ExportWhat::Set(set) if self.sets.contains(set) => {
                ExportFormat::of_kind(ExportKind::Outline)
            }
            ExportWhat::SheetMetal(_) | ExportWhat::Fold(_) | ExportWhat::EveryPart
                if self.patterns.iter().any(|p| &p.what() == what) =>
            {
                ExportFormat::of_kind(ExportKind::Outline)
            }
            _ => Vec::new(),
        }
    }

    /// `what` can be exported from this document: some format holds it.
    pub fn offers(&self, what: &ExportWhat) -> bool {
        !self.formats(what).is_empty()
    }

    /// What the dialog chooses when it opens: the sheet or pattern chosen
    /// last for this document when it still has it, else the selection
    /// when a format holds it, else the model.
    pub fn first_choice(&self, remembered: Option<&ExportWhat>) -> ExportWhat {
        if let Some(what) = remembered.filter(|w| w.is_outline_only() && self.offers(w)) {
            return what.clone();
        }
        if self.offers(&ExportWhat::Selection) {
            return ExportWhat::Selection;
        }
        ExportWhat::Model
    }

    /// What the dialog calls `what`.
    pub fn label(&self, what: &ExportWhat) -> String {
        match what {
            ExportWhat::Model => "The model".to_string(),
            ExportWhat::Selection => match &self.selection {
                Some(selection) if !selection.summary.is_empty() => {
                    format!("The selection ({})", selection.summary)
                }
                _ => "The selection".to_string(),
            },
            ExportWhat::Set(set) => format!("Sheet of set: {set}"),
            ExportWhat::SheetMetal(part) => format!("{} {part}", PatternKind::SheetMetal.label()),
            ExportWhat::Fold(fold) => format!("{} {fold}", PatternKind::Fold.label()),
            ExportWhat::EveryPart => format!("{} every part", PatternKind::EveryPart.label()),
        }
    }
}

/// What the dialog says of `what` under the pointer.
pub fn what_hint(what: &ExportWhat) -> &'static str {
    match what {
        ExportWhat::Model => {
            "Every body as STEP or a mesh; as DXF or SVG a drawing of the model as seen"
        }
        ExportWhat::Selection => {
            "The selected bodies as STEP or a mesh; the selected flat faces, sketch or sheet-metal part as DXF or SVG"
        }
        ExportWhat::Set(_) => {
            "The set's flat faces laid out side by side on one sheet, at true size"
        }
        ExportWhat::SheetMetal(_) => {
            "The part's flat pattern at true size: the outline to cut and the bend lines"
        }
        ExportWhat::Fold(_) => "The fold's crease pattern at true size",
        ExportWhat::EveryPart => {
            "Every sheet-metal part's flat pattern side by side on one sheet, at true size"
        }
    }
}

/// [`ExportChoices`] for the active document and what is selected now.
pub fn export_choices(session: &mut Session) -> ExportChoices {
    let selection = guarded(session, "select.get", &json!({ "context": false }))
        .ok()
        .filter(|s| field(s, "items").as_array().is_some_and(|i| !i.is_empty()))
        .map(|s| {
            let summary = field(&s, "summary")
                .as_str()
                .unwrap_or_default()
                .to_string();
            // One name of each kind asks what the selection fits.
            let formats: Vec<ExportFormat> = [ExportKind::Bodies, ExportKind::Outline]
                .into_iter()
                .filter(|kind| {
                    let probe = match kind {
                        ExportKind::Bodies => "x.step",
                        ExportKind::Outline => "x.dxf",
                    };
                    export_selection(session, probe).is_ok()
                })
                .flat_map(ExportFormat::of_kind)
                .collect();
            let problem = formats.is_empty().then(|| {
                format!(
                    "The selection ({summary}) cannot be exported: select only bodies (STEP or a mesh), or flat faces, one sketch or the body of one sheet-metal part or fold (DXF or SVG); or export the model"
                )
            });
            SelectionChoice {
                summary,
                formats,
                problem,
            }
        });
    ExportChoices {
        selection,
        sets: face_sets(session),
        patterns: flat_patterns(session),
    }
}

/// The named sets of the active document that may hold faces (their kind
/// is face, or a list with face, or every kind), in their order; none
/// when there is no document or the list cannot be read.
pub fn face_sets(session: &mut Session) -> Vec<String> {
    let Ok(list) = guarded(session, "set.list", &json!({})) else {
        return Vec::new();
    };
    let holds_faces = |kind: Option<&Value>| match kind {
        None | Some(Value::Null) => true,
        Some(Value::String(k)) => k == "face",
        Some(Value::Array(kinds)) => kinds.iter().any(|k| k == "face"),
        Some(_) => false,
    };
    list.get("sets")
        .and_then(Value::as_array)
        .map(|sets| {
            sets.iter()
                .filter(|set| holds_faces(set.get("kind")))
                .filter_map(|set| set.get("name").and_then(Value::as_str))
                .take(256)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The text of an error without the command's name in front of it
/// (`file.export: a .step file …` → `A .step file …`), for a person.
pub fn plain_message(text: &str) -> String {
    let mut rest = text.trim();
    // `file.export: file.export: …`: a command's id from the registry,
    // a few deep; any other text before a colon is the message's own.
    for _ in 0..4 {
        match rest.split_once(": ") {
            Some((head, tail)) if head.contains('.') && Session::command(head).is_some() => {
                rest = tail.trim_start();
            }
            _ => break,
        }
    }
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text.to_string(),
    }
}

/// Nothing is selected in the active document (or it cannot be read).
fn selection_is_empty(session: &mut Session) -> bool {
    guarded(session, "select.get", &json!({ "context": false }))
        .ok()
        .and_then(|s| s.get("items").and_then(Value::as_array).map(Vec::is_empty))
        .unwrap_or(true)
}

/// What an export straight to a file of a name (no dialog) holds: the
/// selection, or the model when nothing is selected.
pub fn following_selection(session: &mut Session) -> ExportWhat {
    if selection_is_empty(session) {
        ExportWhat::Model
    } else {
        ExportWhat::Selection
    }
}

/// The sheet's width and gap as `file.export` takes them, those typed.
fn sheet_params(plan: &ExportPlan, params: &mut Vec<(&'static str, Value)>) {
    for (key, text) in [("width", &plan.width), ("gap", &plan.gap)] {
        let text = text.trim();
        if !text.is_empty() {
            params.push((key, Value::String(text.to_string())));
        }
    }
}

/// The parameters of a drawing of the model as seen.
fn drawing_params(drawing: DrawingPlan, params: &mut Vec<(&'static str, Value)>) {
    params.push(("view", Value::String(drawing.view.name().to_string())));
    let projection = if drawing.perspective {
        "perspective"
    } else {
        "orthographic"
    };
    params.push(("projection", Value::String(projection.to_string())));
    let hidden = if drawing.dashed { "dashed" } else { "omit" };
    params.push(("hidden", Value::String(hidden.to_string())));
}

/// The parameters `file.export` is given besides `path` for an export
/// to `name` (whose extension is the format) as `plan` says: the model,
/// the selection's, a sheet or a pattern, and the quality of a mesh. An
/// error when the format cannot hold what was chosen (a selection that
/// does not fit, a pattern to a STEP file): never something else in its
/// place. A name of no export format gets nothing: `file.export` refuses
/// it, naming the formats.
pub fn export_params(
    session: &mut Session,
    name: &str,
    plan: &ExportPlan,
) -> Result<Vec<(&'static str, Value)>, String> {
    let mut params = Vec::new();
    let Some(format) = ExportFormat::of_name(name) else {
        return Ok(params);
    };
    let outline = format.kind() == ExportKind::Outline;
    if plan.what.is_outline_only() && !outline {
        return Err(format!(
            "file.export: a .{} file holds bodies; a sheet or a flat pattern goes into a .dxf or .svg file",
            format.extension()
        ));
    }
    match &plan.what {
        // The set's flat faces on one sheet, whatever is selected.
        ExportWhat::Set(set) => {
            params.push(("set", Value::String(set.clone())));
            sheet_params(plan, &mut params);
        }
        ExportWhat::SheetMetal(part) => params.push(("sheetMetal", Value::String(part.clone()))),
        ExportWhat::Fold(fold) => params.push(("fold", Value::String(fold.clone()))),
        // Every part built now, on one sheet.
        ExportWhat::EveryPart => {
            let parts: Vec<Value> = flat_patterns(session)
                .into_iter()
                .filter(|p| p.kind == PatternKind::SheetMetal)
                .map(|p| Value::String(p.feature))
                .collect();
            params.push(("sheetMetal", Value::Array(parts)));
            sheet_params(plan, &mut params);
        }
        // Every body, or the drawing as the dialog says, whatever is
        // selected.
        ExportWhat::Model => {
            if outline {
                drawing_params(plan.drawing, &mut params);
            }
        }
        ExportWhat::Selection => {
            // Nothing selected (any more): refused, never the model in its
            // place. The hosts make the parameters when Export… is
            // clicked, so a selection changed while the save dialog is
            // open does not change the file.
            if selection_is_empty(session) {
                return Err(
                    "file.export: nothing is selected: select what to export, or export the model"
                        .to_string(),
                );
            }
            match export_selection(session, name)? {
                Some(("view", _)) => drawing_params(plan.drawing, &mut params),
                Some((key, value)) => params.push((key, value)),
                None => {}
            }
        }
    }
    if format.is_mesh() {
        params.push(("quality", Value::String(plan.quality.name().to_string())));
    }
    Ok(params)
}

#[cfg(test)]
mod tests {

    /// The name a file is opened under, for the status bar.
    #[test]
    fn a_file_is_named_by_its_stem() {
        assert_eq!(
            file_stem("/jigs/welding-table.3dcraftjig").as_deref(),
            Some("welding-table")
        );
        assert_eq!(
            file_stem("C:\\models\\plate.3dcraft").as_deref(),
            Some("plate")
        );
        assert_eq!(file_stem("part.step").as_deref(), Some("part.step"));
        assert_eq!(file_stem("/"), None);
    }

    use super::*;

    #[test]
    fn messages_lose_the_command_in_front() {
        assert_eq!(
            plain_message("file.export: a .step file holds bodies"),
            "A .step file holds bodies"
        );
        assert_eq!(
            plain_message("file.open: file.open: no such file: x"),
            "No such file: x"
        );
        assert_eq!(plain_message("Disk full"), "Disk full");
        // Only a command's id is taken off, not any word before a colon.
        assert_eq!(plain_message("download: blocked"), "Download: blocked");
        assert_eq!(
            plain_message("part.step: not found"),
            "Part.step: not found"
        );
        assert_eq!(plain_message(""), "");
        assert_eq!(
            plain_message("the path C:\\a: is odd"),
            "The path C:\\a: is odd"
        );
    }

    #[test]
    fn the_export_choices_and_params_follow_the_selection() {
        let mut session = Session::new(craft3d_engine::Workspace::memory());
        guarded(&mut session, "file.new", &json!({})).unwrap();
        guarded(&mut session, "feature.block", &json!({"size": [1, 2, 3]})).unwrap();
        guarded(
            &mut session,
            "feature.block",
            &json!({"size": [1, 1, 1], "position": [5, 0, 0]}),
        )
        .unwrap();
        // Nothing selected: the model only, in every format.
        let choices = export_choices(&mut session);
        assert_eq!(choices.selection, None);
        assert_eq!(choices.whats(), vec![ExportWhat::Model]);
        assert_eq!(
            choices.formats(&ExportWhat::Model),
            ExportFormat::ALL.to_vec()
        );
        assert_eq!(choices.first_choice(None), ExportWhat::Model);
        let plan = ExportPlan {
            quality: ExportQuality::Fine,
            ..ExportPlan::default()
        };
        assert_eq!(
            export_params(&mut session, "a.stl", &plan).unwrap(),
            vec![("quality", json!("fine"))]
        );
        assert!(
            export_params(&mut session, "a.step", &plan)
                .unwrap()
                .is_empty()
        );
        // A face selected: offered for DXF and SVG only; STEP cannot hold
        // it as the selection, and the model is every body.
        guarded(
            &mut session,
            "select.set",
            &json!({"items": [{"kind": "face", "name": "F1:face(0)", "feature": "F1"}]}),
        )
        .unwrap();
        let choices = export_choices(&mut session);
        let selection = choices.selection.clone().unwrap();
        assert_eq!(
            selection.formats,
            vec![ExportFormat::Dxf, ExportFormat::Svg]
        );
        assert_eq!(selection.problem, None);
        assert_eq!(choices.first_choice(None), ExportWhat::Selection);
        assert_eq!(
            choices.label(&ExportWhat::Selection),
            "The selection (1 face)"
        );
        let chosen = ExportPlan {
            what: ExportWhat::Selection,
            ..plan.clone()
        };
        let refused = export_params(&mut session, "a.step", &chosen).unwrap_err();
        assert!(refused.contains("a .step file holds bodies"), "{refused}");
        assert!(
            export_params(&mut session, "a.step", &plan)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            export_params(&mut session, "a.dxf", &chosen).unwrap(),
            vec![("faces", json!([{"face": "F1:face(0)", "feature": "F1"}]))]
        );
        // The model to DXF or SVG: a drawing of it as seen, whatever is
        // selected, with the view, projection and hidden lines asked.
        assert_eq!(
            export_params(&mut session, "a.dxf", &plan).unwrap(),
            vec![
                ("view", json!("current")),
                ("projection", json!("orthographic")),
                ("hidden", json!("omit")),
            ]
        );
        guarded(&mut session, "select.set", &json!({"mode": "clear"})).unwrap();
        let drawn = ExportPlan {
            drawing: DrawingPlan {
                view: DrawingView::Front,
                perspective: true,
                dashed: true,
            },
            ..plan.clone()
        };
        assert_eq!(
            export_params(&mut session, "a.svg", &drawn).unwrap(),
            vec![
                ("view", json!("front")),
                ("projection", json!("perspective")),
                ("hidden", json!("dashed")),
            ]
        );
        // The selection chosen, but nothing selected (any more): refused,
        // never the model in its place (review of r1-exportflow: before,
        // an emptied selection exported the model).
        let drawn_selection = ExportPlan {
            what: ExportWhat::Selection,
            ..drawn.clone()
        };
        for name in ["a.svg", "a.step"] {
            let refused = export_params(&mut session, name, &drawn_selection).unwrap_err();
            assert!(refused.contains("nothing is selected"), "{refused}");
        }
        assert_eq!(following_selection(&mut session), ExportWhat::Model);
        // An edge selected: no format holds it; said, and the model is
        // still offered.
        guarded(
            &mut session,
            "select.query",
            &json!({"kind": "edge", "limit": 1, "then": "replace"}),
        )
        .unwrap();
        let choices = export_choices(&mut session);
        let selection = choices.selection.clone().unwrap();
        assert!(selection.formats.is_empty());
        assert!(
            selection
                .problem
                .as_deref()
                .unwrap()
                .starts_with("The selection (1 edge) cannot be exported"),
            "{selection:?}"
        );
        assert!(!choices.offers(&ExportWhat::Selection));
        assert_eq!(
            choices.whats(),
            vec![ExportWhat::Model, ExportWhat::Selection]
        );
        assert_eq!(choices.first_choice(None), ExportWhat::Model);
        assert!(export_params(&mut session, "a.dxf", &chosen).is_err());
        // One body of two selected: those bodies, and the quality; not a
        // drawing.
        guarded(
            &mut session,
            "select.set",
            &json!({"items": [{"kind": "body", "feature": "F2"}]}),
        )
        .unwrap();
        let choices = export_choices(&mut session);
        assert_eq!(
            choices.formats(&ExportWhat::Selection),
            ExportFormat::of_kind(ExportKind::Bodies)
        );
        assert_eq!(
            export_params(&mut session, "a.3mf", &chosen).unwrap(),
            vec![("bodies", json!([2])), ("quality", json!("fine"))]
        );
        // A name of no format: nothing; file.export names the formats.
        assert!(
            export_params(&mut session, "a.txt", &chosen)
                .unwrap()
                .is_empty()
        );
    }

    /// The export dialog offers the document's named sets that may hold
    /// faces; a sheet of one is `file.export {set, width, gap}` whatever
    /// is selected, and writes the file (R1 A6).
    #[test]
    fn a_named_set_is_offered_as_a_sheet_for_an_outline() {
        let mut session = Session::new(craft3d_engine::Workspace::memory());
        guarded(&mut session, "file.new", &json!({"units": "mm"})).unwrap();
        guarded(&mut session, "feature.block", &json!({"size": [10, 20, 3]})).unwrap();
        guarded(
            &mut session,
            "feature.block",
            &json!({"size": [30, 10, 3], "position": [40, 0, 0]}),
        )
        .unwrap();
        // Without sets nothing changes.
        assert!(export_choices(&mut session).sets.is_empty());
        for (name, kind) in [("Cut sheet", json!("face")), ("Parts", json!("body"))] {
            guarded(
                &mut session,
                "set.define",
                &json!({"name": name, "kind": kind, "where": {"normal": {"along": "z"}}}),
            )
            .unwrap();
        }
        guarded(
            &mut session,
            "set.define",
            &json!({"name": "Mixed", "kind": ["edge", "face"], "where": {"normal": {"along": "z"}}}),
        )
        .unwrap();
        assert_eq!(face_sets(&mut session), vec!["Cut sheet", "Mixed"]);
        // An edge selected: the sets are offered all the same, for DXF and
        // SVG only.
        guarded(
            &mut session,
            "select.query",
            &json!({"kind": "edge", "limit": 1, "then": "replace"}),
        )
        .unwrap();
        let choices = export_choices(&mut session);
        assert_eq!(choices.sets, vec!["Cut sheet", "Mixed"]);
        let sheet = ExportWhat::Set("Cut sheet".into());
        assert_eq!(
            choices.formats(&sheet),
            vec![ExportFormat::Dxf, ExportFormat::Svg]
        );
        assert!(!choices.offers(&ExportWhat::Set("Parts".into())));
        assert_eq!(choices.label(&sheet), "Sheet of set: Cut sheet");
        // Chosen last for this document: chosen again.
        assert_eq!(choices.first_choice(Some(&sheet)), sheet);
        assert_eq!(
            choices.first_choice(Some(&ExportWhat::Set("Gone".into()))),
            ExportWhat::Model
        );
        let plan = ExportPlan {
            what: sheet.clone(),
            width: " 40 mm ".into(),
            ..ExportPlan::default()
        };
        let params = export_params(&mut session, "a.svg", &plan).unwrap();
        assert_eq!(
            params,
            vec![("set", json!("Cut sheet")), ("width", json!("40 mm"))]
        );
        let mut call = json!({"path": "a.svg"});
        for (key, value) in params {
            call[key] = value;
        }
        let written = guarded(&mut session, "file.export", &call).unwrap();
        assert!(session.take_file("a.svg").is_some(), "{written}");
        // A sheet is no file of bodies: refused, never every body instead
        // (before 2026-10-27 a mesh ignored the sheet; the dialog no
        // longer offers that pair at all). A regression test of a changed
        // expectation: it has no oracle independent of the code (the rule
        // "never something else in its place" is the reason; testing.md,
        // *Changing an expectation*, condition 3; logged in the dev log).
        let refused = export_params(&mut session, "a.stl", &plan).unwrap_err();
        assert!(refused.contains("a .stl file holds bodies"), "{refused}");
    }

    /// The export dialog offers the flat pattern of every sheet-metal
    /// part and the crease pattern of every fold, whatever is selected,
    /// and a part's or fold's body selected is its pattern
    /// (plan/repro/repro-sheet-metal-enclosure-1: the dialog never reached
    /// `file.export {sheetMetal}`).
    #[test]
    fn a_flat_pattern_is_offered_for_an_outline() {
        let mut session = Session::new(craft3d_engine::Workspace::memory());
        guarded(&mut session, "file.new", &json!({"units": "mm"})).unwrap();
        // Without a part or a fold nothing is offered.
        assert!(export_choices(&mut session).patterns.is_empty());
        guarded(
            &mut session,
            "sketch.create",
            &json!({"plane": "xy", "name": "S"}),
        )
        .unwrap();
        guarded(
            &mut session,
            "sketch.rect",
            &json!({"sketch": "S", "from": [0, 0], "to": [50, 40]}),
        )
        .unwrap();
        guarded(
            &mut session,
            "feature.sheetMetal",
            &json!({"sketch": "S", "name": "Part", "thickness": "1 mm", "radius": "1 mm",
                "kFactor": 0.4, "flanges": [{"edge": "e6", "length": "15 mm", "angle": "90 deg"}]}),
        )
        .unwrap();
        // A fold of a square with one crease.
        guarded(&mut session, "sketch.create", &json!({"name": "Paper"})).unwrap();
        guarded(
            &mut session,
            "sketch.polyline",
            &json!({"sketch": "Paper", "points": [[100, 0], [140, 0], [140, 40], [100, 40]], "closed": true}),
        )
        .unwrap();
        let crease = guarded(
            &mut session,
            "sketch.line",
            &json!({"sketch": "Paper", "from": [100, 20], "to": [140, 20]}),
        )
        .unwrap();
        guarded(
            &mut session,
            "feature.fold",
            &json!({"sketch": "Paper", "name": "Crane", "creases": [
                {"line": crease["entity"], "kind": "valley", "angle": "90 deg"}]}),
        )
        .unwrap();
        let choices = export_choices(&mut session);
        let labels: Vec<String> = choices.patterns.iter().map(FlatPattern::label).collect();
        assert_eq!(
            labels,
            vec!["Flat pattern of Part", "Crease pattern of Crane"]
        );
        let whats: Vec<String> = choices.whats().iter().map(|w| choices.label(w)).collect();
        assert_eq!(
            whats,
            vec![
                "The model",
                "Flat pattern of Part",
                "Crease pattern of Crane"
            ]
        );
        // The part's body selected: its flat pattern, as the command.
        guarded(
            &mut session,
            "select.set",
            &json!({"items": [{"kind": "body", "name": "Part"}]}),
        )
        .unwrap();
        assert_eq!(
            export_selection(&mut session, "a.svg").unwrap(),
            Some(("sheetMetal", json!("Part")))
        );
        let choices = export_choices(&mut session);
        assert_eq!(
            choices.formats(&ExportWhat::Selection),
            ExportFormat::ALL.to_vec(),
            "the part as a body, or its flat pattern"
        );
        let selection = ExportPlan {
            what: ExportWhat::Selection,
            ..ExportPlan::default()
        };
        assert_eq!(
            export_params(&mut session, "a.dxf", &selection).unwrap(),
            vec![("sheetMetal", json!("Part"))]
        );
        // An edge selected: each pattern is offered and written whatever
        // is selected.
        guarded(
            &mut session,
            "select.query",
            &json!({"kind": "edge", "limit": 1, "then": "replace"}),
        )
        .unwrap();
        let choices = export_choices(&mut session);
        assert!(!choices.offers(&ExportWhat::Selection));
        for (pattern, key) in choices.patterns.iter().zip(["sheetMetal", "fold"]) {
            let plan = ExportPlan {
                what: pattern.what(),
                ..ExportPlan::default()
            };
            assert!(choices.offers(&plan.what));
            assert_eq!(choices.first_choice(Some(&plan.what)), plan.what);
            for file in ["a.svg", "a.dxf"] {
                let params = export_params(&mut session, file, &plan).unwrap();
                assert_eq!(params, vec![(key, json!(pattern.feature))]);
                let mut call = json!({"path": file});
                for (key, value) in params {
                    call[key] = value;
                }
                let written = guarded(&mut session, "file.export", &call).unwrap();
                let bytes = session.take_file(file).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                if key == "sheetMetal" {
                    assert!(text.contains("bendUp"), "{written} {text}");
                }
            }
            // A pattern is no file of bodies (before 2026-10-27 a mesh
            // ignored it). A regression test of a changed expectation with
            // no independent oracle (testing.md, *Changing an expectation*,
            // condition 3; logged in the dev log).
            assert!(export_params(&mut session, "a.stl", &plan).is_err());
        }
        // A second part: every part's flat pattern on one sheet.
        guarded(
            &mut session,
            "sketch.create",
            &json!({"plane": "xy", "name": "S2"}),
        )
        .unwrap();
        guarded(
            &mut session,
            "sketch.rect",
            &json!({"sketch": "S2", "from": [0, 100], "to": [30, 130]}),
        )
        .unwrap();
        guarded(
            &mut session,
            "feature.sheetMetal",
            &json!({"sketch": "S2", "name": "Lid", "thickness": "1 mm", "radius": "1 mm",
                "kFactor": 0.4, "flanges": [{"edge": "e5", "length": "10 mm", "angle": "90 deg"}]}),
        )
        .unwrap();
        let choices = export_choices(&mut session);
        let every = choices.patterns.last().cloned().unwrap();
        assert_eq!(every.label(), "Flat patterns of every part");
        assert_eq!(every.what(), ExportWhat::EveryPart);
        assert_eq!(
            choices.label(&ExportWhat::EveryPart),
            "Flat patterns of every part"
        );
        let plan = ExportPlan {
            what: ExportWhat::EveryPart,
            width: "200 mm".into(),
            ..ExportPlan::default()
        };
        let params = export_params(&mut session, "a.dxf", &plan).unwrap();
        assert_eq!(
            params,
            vec![
                ("sheetMetal", json!(["Part", "Lid"])),
                ("width", json!("200 mm"))
            ]
        );
        let mut call = json!({"path": "a.dxf"});
        for (key, value) in params {
            call[key] = value;
        }
        let written = guarded(&mut session, "file.export", &call).unwrap();
        assert_eq!(
            written["sheetMetal"].as_array().map(Vec::len),
            Some(2),
            "{written}"
        );
        assert!(session.take_file("a.dxf").is_some());
        // A pattern of a part the document does not have is not offered.
        let gone = ExportWhat::SheetMetal("Gone".into());
        assert!(!choices.offers(&gone));
        assert_eq!(choices.first_choice(Some(&gone)), ExportWhat::Model);
    }

    /// A name typed without the format's extension, or with another
    /// format's, is named for the format chosen; any other extension is
    /// kept and the format's appended (the developer, 2026-10-27: STL
    /// bytes were written under `.dxf`... never again).
    #[test]
    fn a_file_name_gets_the_extension_of_its_format() {
        let cases = [
            ("part", ExportFormat::Stl, "part.stl"),
            ("part.stl", ExportFormat::Stl, "part.stl"),
            ("Part.STL", ExportFormat::Stl, "Part.STL"),
            ("part.stl", ExportFormat::Dxf, "part.dxf"),
            ("part.dxf", ExportFormat::Stl, "part.stl"),
            ("part.stp", ExportFormat::Step, "part.stp"),
            ("part.step", ExportFormat::ThreeMf, "part.3mf"),
            ("part.3dcraft", ExportFormat::Svg, "part.svg"),
            ("box.3dcraftjig", ExportFormat::Obj, "box.obj"),
            ("bracket.v2", ExportFormat::Step, "bracket.v2.step"),
            ("part.", ExportFormat::Svg, "part.svg"),
            ("  ", ExportFormat::Obj, "untitled.obj"),
            ("", ExportFormat::Step, "untitled.step"),
            (".stl", ExportFormat::Dxf, ".stl.dxf"),
            ("a.b.dxf", ExportFormat::Svg, "a.b.svg"),
            ("ünï.cöde", ExportFormat::Stl, "ünï.cöde.stl"),
        ];
        for (name, format, wanted) in cases {
            let named = export_file_name(name, format);
            assert_eq!(named, wanted, "{name:?} as {format:?}");
            assert_eq!(ExportFormat::of_name(&named), Some(format), "{named}");
            assert_eq!(export_file_name(&named, format), named, "settled");
        }
        for format in ExportFormat::ALL {
            assert_eq!(
                ExportFormat::from_extension(format.extension()),
                Some(format)
            );
            assert_eq!(
                serde_json::to_value(format).unwrap(),
                json!(format.extension())
            );
            assert_eq!(
                suggested_export_name(Some("/m/gear.3dcraft"), Some(format.extension())),
                format!("gear.{}", format.extension())
            );
        }
        assert_eq!(
            ExportFormat::from_extension("STP"),
            Some(ExportFormat::Step)
        );
        assert_eq!(ExportFormat::from_extension("txt"), None);
    }

    #[test]
    fn names_are_proposed_from_the_document() {
        assert_eq!(suggested_name(None), "untitled.3dcraft");
        assert_eq!(suggested_name(Some("jigs/box.3dcraftjig")), "box.3dcraft");
        assert_eq!(suggested_jig_name(Some("w/box.3dcraft")), "box.3dcraftjig");
        assert_eq!(suggested_jig_name(None), "untitled.3dcraftjig");
        assert_eq!(suggested_name(Some("parts/a.3dcraft")), "a.3dcraft");
        assert_eq!(suggested_export_name(None, None), "untitled.step");
        assert_eq!(
            suggested_export_name(Some("a.3dcraft"), Some("stl")),
            "a.stl"
        );
    }

    #[test]
    fn the_extension_says_what_an_export_holds() {
        assert_eq!(export_kind("a.STEP"), Some(ExportKind::Bodies));
        assert_eq!(export_kind("a.3mf"), Some(ExportKind::Bodies));
        assert_eq!(export_kind("a.svg"), Some(ExportKind::Outline));
        assert_eq!(export_kind("a.txt"), None);
        assert_eq!(export_kind("a"), None);
        for (_, extensions) in EXPORT_FORMATS {
            for extension in extensions {
                assert!(export_kind(&format!("x.{extension}")).is_some());
            }
        }
    }
}
