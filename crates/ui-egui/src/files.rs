//! What the hosts' file actions share, whether the file is on disk (the
//! desktop's dialogs) or in the session's memory (the browser): the
//! names they propose, and what `file.export` is given for the window's
//! selection. Hosts run the commands; nothing here touches a file.

use craft3d_engine::{CommandError, Session};
use serde_json::{Value, json};
use std::path::Path;

/// The extension of the native format.
pub const EXTENSION: &str = "3dcraft";

/// The extension of a published Jig: the same format, opened to be used.
pub const JIG_EXTENSION: &str = "3dcraftjig";

/// The extensions Open reads: a document and a published Jig.
pub const OPEN_EXTENSIONS: [&str; 2] = [EXTENSION, JIG_EXTENSION];

/// The extensions `file.import` reads (STEP), in both cases for the
/// dialogs that filter by case.
pub const IMPORT_EXTENSIONS: [&str; 4] = ["step", "stp", "STEP", "STP"];

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
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

/// The view a drawing of the model as seen is made from (DXF or SVG with
/// nothing selected; `file.export {view}`), as the export dialog offers
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DrawingPlan {
    pub view: DrawingView,
    /// Perspective (an illustration) instead of orthographic (to scale).
    pub perspective: bool,
    /// The hidden lines drawn dashed instead of left out.
    pub dashed: bool,
}

/// A sheet of a named set's flat faces (DXF or SVG; `file.export {set,
/// width, gap}`), as the export dialog offers it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SheetPlan {
    /// The named set whose flat faces go on the sheet; none for the
    /// selection or the view instead.
    pub set: Option<String>,
    /// The sheet's width as typed (`600 mm`, `600` in the document's
    /// unit); empty for no limit.
    pub width: String,
    /// The gap between two outlines as typed; empty for 5 mm.
    pub gap: String,
    /// The flat pattern of a sheet-metal part or the crease pattern of a
    /// fold instead of the selection, the view or a set (`file.export
    /// {sheetMetal}` or `{fold}`); a set and a pattern are not both chosen.
    pub pattern: Option<FlatPattern>,
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
}

/// The sheet-metal parts and folds of the active document, in history
/// order (the parts first), and with two parts or more every part on one
/// sheet: each a pattern a DXF or SVG export can draw. None when there is
/// no document or the query fails.
pub fn flat_patterns(session: &mut Session) -> Vec<FlatPattern> {
    let mut out = Vec::new();
    for kind in PatternKind::ALL {
        let Ok(found) = guarded(
            session,
            "select.query",
            &json!({"kind": "feature", "where": {"type": kind.param()}, "limit": 256}),
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

/// Longest width or gap the export dialog keeps, in characters.
const MAX_SHEET_TEXT: usize = 200;

/// How the person chose to export, in the export dialog.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExportPlan {
    /// How finely a mesh follows the model (STL, 3MF, OBJ).
    pub quality: ExportQuality,
    /// Every body, whatever is selected: the way out the dialog offers
    /// when the selection does not fit the format.
    pub all_bodies: bool,
    /// A drawing of the model as seen (DXF, SVG with nothing selected).
    pub drawing: DrawingPlan,
    /// A sheet of a named set's flat faces instead of the selection or
    /// the view (DXF, SVG).
    pub sheet: SheetPlan,
}

/// Whether a file of this name is a mesh (`quality` applies).
pub fn is_mesh(name: &str) -> bool {
    matches!(extension_of(name).as_deref(), Some("stl" | "3mf" | "obj"))
}

/// What the export dialog knows before an export to `name`, from the
/// window's selection ([`export_selection`]): whether it is a mesh, and
/// why the selection cannot go into the file, if it cannot.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExportCheck {
    pub kind: Option<ExportKind>,
    pub mesh: bool,
    /// A drawing of the model as seen: DXF or SVG with nothing selected.
    pub drawing: bool,
    /// Why the selection does not fit the format, said for the person.
    pub problem: Option<String>,
    /// The document's named sets that may hold faces (DXF or SVG), in
    /// their order: each can be laid out as a sheet instead.
    pub sets: Vec<String>,
    /// The document's sheet-metal parts and folds (DXF or SVG): the flat
    /// or crease pattern of each can be drawn instead.
    pub patterns: Vec<FlatPattern>,
}

impl ExportCheck {
    /// Every body can be exported instead of the selection: a STEP or
    /// mesh file whose selection does not fit.
    pub fn offers_all_bodies(&self) -> bool {
        self.kind == Some(ExportKind::Bodies) && self.problem.is_some()
    }

    /// The export dialog is to be shown before the file is written: a
    /// mesh (its quality), a selection that does not fit a STEP file, or
    /// a drawing of the model as seen (its view).
    /// Named sets are offered in the dialog when it is shown for an
    /// outline that fits (a selection that does not is said at once), or
    /// when the sheet chosen last is one of them ([`Self::offers_sheet`]).
    /// An outline of a document with a sheet-metal part or a fold always
    /// asks, so its pattern can be chosen whatever is selected.
    pub fn asks(&self) -> bool {
        self.mesh
            || self.offers_all_bodies()
            || self.drawing
            || (!self.sets.is_empty() && self.problem.is_none())
            || !self.patterns.is_empty()
    }

    /// The sheet `sheet` names one of this export's sets: the dialog is
    /// shown with it chosen, even for a selection the outline cannot hold.
    pub fn offers_sheet(&self, sheet: &SheetPlan) -> bool {
        sheet
            .set
            .as_deref()
            .is_some_and(|set| self.sets.iter().any(|s| s == set))
            || sheet
                .pattern
                .as_ref()
                .is_some_and(|p| self.patterns.contains(p))
    }

    /// The pattern `plan` draws, when it is one of this export's.
    pub fn sheet_pattern<'a>(&self, plan: &'a ExportPlan) -> Option<&'a FlatPattern> {
        plan.sheet
            .pattern
            .as_ref()
            .filter(|p| self.patterns.contains(p))
    }

    /// The set `plan` lays out as a sheet, when it is one of this
    /// export's (an outline with named sets).
    pub fn sheet_set<'a>(&self, plan: &'a ExportPlan) -> Option<&'a str> {
        plan.sheet
            .set
            .as_deref()
            .filter(|set| self.sets.iter().any(|s| s == set))
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

/// [`ExportCheck`] for an export to `name` with what is selected now.
pub fn export_check(session: &mut Session, name: &str) -> ExportCheck {
    let kind = export_kind(name);
    let selection = export_selection(session, name);
    let drawing = matches!(selection, Ok(Some(("view", _))));
    let problem = selection.err().map(|why| plain_message(&why));
    let (sets, patterns) = if kind == Some(ExportKind::Outline) {
        (face_sets(session), flat_patterns(session))
    } else {
        (Vec::new(), Vec::new())
    };
    ExportCheck {
        kind,
        mesh: is_mesh(name),
        drawing,
        problem,
        sets,
        patterns,
    }
}

/// The parameters `file.export` is given besides `path` for an export
/// to `name` as `plan` says: the selection's (unless every body was
/// chosen) and the quality of a mesh. An error when the selection does
/// not fit and every body was not chosen.
pub fn export_params(
    session: &mut Session,
    name: &str,
    plan: &ExportPlan,
) -> Result<Vec<(&'static str, Value)>, String> {
    let mut params = Vec::new();
    if export_kind(name) == Some(ExportKind::Outline)
        && let Some(pattern) = &plan.sheet.pattern
    {
        // The part's flat pattern or the fold's crease pattern, whatever
        // is selected; every part's on one sheet, with its width and gap.
        if pattern.kind == PatternKind::EveryPart {
            let parts: Vec<Value> = flat_patterns(session)
                .into_iter()
                .filter(|p| p.kind == PatternKind::SheetMetal)
                .map(|p| Value::String(p.feature))
                .collect();
            params.push(("sheetMetal", Value::Array(parts)));
            for (key, text) in [("width", &plan.sheet.width), ("gap", &plan.sheet.gap)] {
                let text = text.trim();
                if !text.is_empty() {
                    params.push((key, Value::String(text.to_string())));
                }
            }
        } else {
            params.push((pattern.kind.param(), Value::String(pattern.feature.clone())));
        }
        return Ok(params);
    }
    if export_kind(name) == Some(ExportKind::Outline)
        && let Some(set) = plan.sheet.set.as_deref()
    {
        // A sheet of the set's flat faces, whatever is selected.
        params.push(("set", Value::String(set.to_string())));
        for (key, text) in [("width", &plan.sheet.width), ("gap", &plan.sheet.gap)] {
            let text = text.trim();
            if !text.is_empty() {
                params.push((key, Value::String(text.to_string())));
            }
        }
        return Ok(params);
    }
    let all = plan.all_bodies && export_kind(name) == Some(ExportKind::Bodies);
    let selection = if all {
        None
    } else {
        export_selection(session, name)?
    };
    match selection {
        // Nothing selected, to DXF or SVG: the drawing as the dialog says.
        Some(("view", _)) => {
            let drawing = plan.drawing;
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
        Some((key, value)) => params.push((key, value)),
        None => {}
    }
    if is_mesh(name) {
        params.push(("quality", Value::String(plan.quality.name().to_string())));
    }
    Ok(params)
}

/// The quality choice and the selection's warning of the export dialog,
/// for `check`, changing `plan`; the desktop's dialog and the browser's
/// draw the same.
pub fn export_options_ui(ui: &mut egui::Ui, check: &ExportCheck, plan: &mut ExportPlan) {
    // A set chosen last that this document does not have is not offered.
    if check.sheet_set(plan).is_none() {
        plan.sheet.set = None;
    }
    if check.sheet_pattern(plan).is_none() {
        plan.sheet.pattern = None;
    }
    if !check.sets.is_empty() || !check.patterns.is_empty() {
        sheet_choice_ui(ui, check, plan);
        if plan.sheet.set.is_some() || plan.sheet.pattern.is_some() {
            return;
        }
    }
    if check.mesh {
        ui.horizontal(|ui| {
            ui.label("Quality:");
            for quality in ExportQuality::ALL {
                ui.radio_value(&mut plan.quality, quality, quality.label())
                    .on_hover_text(quality.hint());
            }
        });
    }
    if check.drawing {
        drawing_options_ui(ui, &mut plan.drawing);
    }
    if let Some(problem) = &check.problem {
        ui.add(egui::Label::new(problem.as_str()).wrap());
    }
}

/// What goes into a DXF or SVG file when the document has named sets,
/// sheet-metal parts or folds: the selection (or with nothing selected
/// the view), a sheet of a set's flat faces with its width and gap, or a
/// part's flat pattern or a fold's crease pattern.
fn sheet_choice_ui(ui: &mut egui::Ui, check: &ExportCheck, plan: &mut ExportPlan) {
    let own = if check.drawing {
        "The view"
    } else {
        "The selection"
    };
    ui.horizontal_wrapped(|ui| {
        ui.label("What:");
        let neither = plan.sheet.set.is_none() && plan.sheet.pattern.is_none();
        if ui.radio(neither, own).clicked() {
            plan.sheet.set = None;
            plan.sheet.pattern = None;
        }
        for set in &check.sets {
            let chosen = plan.sheet.set.as_deref() == Some(set.as_str());
            if ui
                .radio(chosen, format!("Sheet of set: {set}"))
                .on_hover_text(
                    "The set's flat faces laid out side by side on one sheet, at true size",
                )
                .clicked()
            {
                plan.sheet.set = Some(set.clone());
                plan.sheet.pattern = None;
            }
        }
        for pattern in &check.patterns {
            let chosen = plan.sheet.pattern.as_ref() == Some(pattern);
            let hint = match pattern.kind {
                PatternKind::SheetMetal => {
                    "The part's flat pattern at true size: the outline to cut and the bend lines"
                }
                PatternKind::Fold => "The fold's crease pattern at true size",
                PatternKind::EveryPart => {
                    "Every sheet-metal part's flat pattern side by side on one sheet, at true size"
                }
            };
            if ui
                .radio(chosen, pattern.label())
                .on_hover_text(hint)
                .clicked()
            {
                plan.sheet.pattern = Some(pattern.clone());
                plan.sheet.set = None;
            }
        }
    });
    let every = plan
        .sheet
        .pattern
        .as_ref()
        .is_some_and(|p| p.kind == PatternKind::EveryPart);
    if plan.sheet.set.is_none() && !every {
        return;
    }
    ui.horizontal(|ui| {
        ui.label("Sheet width:");
        ui.add(
            egui::TextEdit::singleline(&mut plan.sheet.width)
                .hint_text("no limit")
                .char_limit(MAX_SHEET_TEXT)
                .desired_width(90.0),
        )
        .on_hover_text("Rows wrap at this width: 600 mm, 24 in; empty for one row");
        ui.label("Gap:");
        ui.add(
            egui::TextEdit::singleline(&mut plan.sheet.gap)
                .hint_text("5 mm")
                .char_limit(MAX_SHEET_TEXT)
                .desired_width(70.0),
        )
        .on_hover_text("Space between two outlines");
    });
}

/// The choices of a drawing of the model as seen: from which view, to
/// scale or in perspective, and what becomes of the hidden lines.
fn drawing_options_ui(ui: &mut egui::Ui, drawing: &mut DrawingPlan) {
    ui.label("A drawing of the model as seen: its edges and outlines.");
    ui.horizontal(|ui| {
        ui.label("View:");
        egui::ComboBox::from_id_salt("craft3d-export-view")
            .selected_text(drawing.view.label())
            .show_ui(ui, |ui| {
                for view in DrawingView::ALL {
                    ui.selectable_value(&mut drawing.view, view, view.label());
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Projection:");
        ui.radio_value(&mut drawing.perspective, false, "Orthographic")
            .on_hover_text("To scale: one unit in the file is one unit of the model");
        ui.radio_value(&mut drawing.perspective, true, "Perspective")
            .on_hover_text("As the eye sees it, for an illustration: not to scale");
    });
    ui.horizontal(|ui| {
        ui.label("Hidden lines:");
        ui.radio_value(&mut drawing.dashed, false, "Leave out");
        ui.radio_value(&mut drawing.dashed, true, "Dashed")
            .on_hover_text("Lines behind a face drawn dashed, on a layer of their own");
    });
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
    fn the_export_check_and_params_follow_the_selection() {
        let mut session = Session::new(craft3d_engine::Workspace::memory());
        guarded(&mut session, "file.new", &json!({})).unwrap();
        guarded(&mut session, "feature.block", &json!({"size": [1, 2, 3]})).unwrap();
        guarded(
            &mut session,
            "feature.block",
            &json!({"size": [1, 1, 1], "position": [5, 0, 0]}),
        )
        .unwrap();
        // Nothing selected: a mesh asks for its quality, STEP nothing.
        let stl = export_check(&mut session, "a.stl");
        assert!(stl.mesh && stl.asks() && stl.problem.is_none());
        let step = export_check(&mut session, "a.step");
        assert!(!step.asks());
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
        // A face selected: STEP cannot hold it; every body instead.
        guarded(
            &mut session,
            "select.set",
            &json!({"items": [{"kind": "face", "name": "F1:face(0)", "feature": "F1"}]}),
        )
        .unwrap();
        let step = export_check(&mut session, "a.step");
        assert!(step.asks() && step.offers_all_bodies(), "{step:?}");
        assert!(
            step.problem.as_deref().unwrap().starts_with("A .step file"),
            "{step:?}"
        );
        assert!(export_params(&mut session, "a.step", &plan).is_err());
        let all = ExportPlan {
            all_bodies: true,
            ..plan.clone()
        };
        assert!(
            export_params(&mut session, "a.step", &all)
                .unwrap()
                .is_empty()
        );
        // Nothing selected, to DXF or SVG: a drawing of the model as seen,
        // whose view, projection and hidden lines the dialog asks for.
        guarded(&mut session, "select.set", &json!({"mode": "clear"})).unwrap();
        let dxf = export_check(&mut session, "a.dxf");
        assert!(dxf.drawing && dxf.asks() && dxf.problem.is_none() && !dxf.offers_all_bodies());
        assert_eq!(
            export_params(&mut session, "a.dxf", &plan).unwrap(),
            vec![
                ("view", json!("current")),
                ("projection", json!("orthographic")),
                ("hidden", json!("omit")),
            ]
        );
        let chosen = ExportPlan {
            drawing: DrawingPlan {
                view: DrawingView::Front,
                perspective: true,
                dashed: true,
            },
            ..plan.clone()
        };
        assert_eq!(
            export_params(&mut session, "a.svg", &chosen).unwrap(),
            vec![
                ("view", json!("front")),
                ("projection", json!("perspective")),
                ("hidden", json!("dashed")),
            ]
        );
        assert!(!export_check(&mut session, "a.stl").drawing);
        // An outline cannot be every body: an edge selected is refused.
        guarded(
            &mut session,
            "select.query",
            &json!({"kind": "edge", "limit": 1, "then": "replace"}),
        )
        .unwrap();
        let dxf = export_check(&mut session, "a.dxf");
        assert!(dxf.problem.is_some() && !dxf.offers_all_bodies() && !dxf.asks());
        assert!(export_params(&mut session, "a.dxf", &all).is_err());
        // One body of two selected: those bodies, and the quality.
        guarded(
            &mut session,
            "select.set",
            &json!({"items": [{"kind": "body", "feature": "F2"}]}),
        )
        .unwrap();
        assert_eq!(
            export_params(&mut session, "a.3mf", &plan).unwrap(),
            vec![("bodies", json!([2])), ("quality", json!("fine"))]
        );
    }

    /// A DXF or SVG export offers the document's named sets that may hold
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
        let dxf = export_check(&mut session, "a.dxf");
        assert!(dxf.sets.is_empty());
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
        // An edge selected: no outline of its own, said at once unless a
        // sheet of a set was chosen last; then the dialog offers it.
        guarded(
            &mut session,
            "select.query",
            &json!({"kind": "edge", "limit": 1, "then": "replace"}),
        )
        .unwrap();
        let svg = export_check(&mut session, "a.svg");
        assert_eq!(svg.sets, vec!["Cut sheet", "Mixed"]);
        assert!(!svg.asks() && svg.problem.is_some(), "{svg:?}");
        assert!(!svg.offers_sheet(&SheetPlan::default()));
        assert!(!svg.offers_sheet(&SheetPlan {
            set: Some("Parts".into()),
            ..SheetPlan::default()
        }));
        assert!(export_check(&mut session, "a.stl").sets.is_empty());
        let mut plan = ExportPlan {
            sheet: SheetPlan {
                set: Some("Cut sheet".into()),
                width: " 40 mm ".into(),
                gap: String::new(),
                pattern: None,
            },
            ..ExportPlan::default()
        };
        assert_eq!(svg.sheet_set(&plan), Some("Cut sheet"));
        assert!(svg.offers_sheet(&plan.sheet));
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
        // A set the document does not have is not offered; a mesh ignores
        // the sheet.
        plan.sheet.set = Some("Gone".into());
        assert_eq!(svg.sheet_set(&plan), None);
        plan.all_bodies = true;
        assert!(
            !export_params(&mut session, "a.stl", &plan)
                .unwrap()
                .iter()
                .any(|(k, _)| *k == "set")
        );
    }

    /// A DXF or SVG export offers the flat pattern of every sheet-metal
    /// part and the crease pattern of every fold, whatever is selected,
    /// and a part's or fold's body selected is its pattern
    /// (plan/repro/repro-sheet-metal-enclosure-1: the dialog never reached
    /// `file.export {sheetMetal}`).
    #[test]
    fn a_flat_pattern_is_offered_for_an_outline() {
        let mut session = Session::new(craft3d_engine::Workspace::memory());
        guarded(&mut session, "file.new", &json!({"units": "mm"})).unwrap();
        // Without a part or a fold nothing is offered.
        assert!(export_check(&mut session, "a.svg").patterns.is_empty());
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
        let svg = export_check(&mut session, "a.svg");
        let labels: Vec<String> = svg.patterns.iter().map(FlatPattern::label).collect();
        assert_eq!(
            labels,
            vec!["Flat pattern of Part", "Crease pattern of Crane"]
        );
        assert!(svg.asks(), "{svg:?}");
        assert!(export_check(&mut session, "a.step").patterns.is_empty());
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
        let none = ExportPlan::default();
        assert_eq!(
            export_params(&mut session, "a.dxf", &none).unwrap(),
            vec![("sheetMetal", json!("Part"))]
        );
        // An edge selected: refused as the selection, but each pattern is
        // offered and written whatever is selected.
        guarded(
            &mut session,
            "select.query",
            &json!({"kind": "edge", "limit": 1, "then": "replace"}),
        )
        .unwrap();
        let svg = export_check(&mut session, "a.svg");
        assert!(svg.problem.is_some() && svg.asks(), "{svg:?}");
        for (pattern, key) in svg.patterns.iter().zip(["sheetMetal", "fold"]) {
            let plan = ExportPlan {
                sheet: SheetPlan {
                    pattern: Some(pattern.clone()),
                    ..SheetPlan::default()
                },
                ..ExportPlan::default()
            };
            assert!(svg.offers_sheet(&plan.sheet));
            assert_eq!(svg.sheet_pattern(&plan), Some(pattern));
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
            // A mesh ignores the pattern.
            assert!(
                !export_params(&mut session, "a.stl", &plan)
                    .unwrap()
                    .iter()
                    .any(|(k, _)| *k == key)
            );
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
        let svg = export_check(&mut session, "a.svg");
        let every = svg.patterns.last().cloned().unwrap();
        assert_eq!(every.label(), "Flat patterns of every part");
        let plan = ExportPlan {
            sheet: SheetPlan {
                pattern: Some(every),
                width: "200 mm".into(),
                ..SheetPlan::default()
            },
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
        let gone = SheetPlan {
            pattern: Some(FlatPattern {
                kind: PatternKind::SheetMetal,
                feature: "Gone".into(),
                id: "F99".into(),
            }),
            ..SheetPlan::default()
        };
        assert!(!svg.offers_sheet(&gone));
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
