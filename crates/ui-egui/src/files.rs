//! What the hosts' file actions share, whether the file is on disk (the
//! desktop's dialogs) or in the session's memory (the browser): the
//! names they propose, and what `file.export` is given for the window's
//! selection. Hosts run the commands; nothing here touches a file.

use craft3d_engine::{CommandError, Session};
use serde_json::{Value, json};
use std::path::Path;

/// The extension of the native format.
pub const EXTENSION: &str = "3dcraft";

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
/// under another name.
pub fn suggested_name(path: Option<&str>) -> String {
    path.and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| format!("untitled.{EXTENSION}"), str::to_string)
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
    /// The flat outline of faces or of a sketch (DXF, SVG).
    Outline,
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
///   file needs faces or a sketch, so it is refused.
/// - Bodies only, to STEP or a mesh file: those bodies (`bodies`, by the
///   ids of their features); every body of the document is as nothing
///   selected.
/// - Faces only, to DXF or SVG: those faces on one sheet (`faces`; a face
///   that is not flat is refused by `file.export`, naming it).
/// - One sketch only, to DXF or SVG: its outline (`sketch`).
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
                return Err(format!(
                    "{ID}: a .{extension} file is the outline of flat faces or of a sketch; select the faces or the sketch to export first"
                ));
            }
            if faces == items.len() {
                let list: Vec<Value> = items
                    .iter()
                    .map(|i| json!({ "face": field(i, "name"), "feature": field(i, "feature") }))
                    .collect();
                return Ok(Some(("faces", Value::Array(list))));
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
                "{ID}: a .{extension} file is the outline of flat faces or of one sketch, and the selection is {summary}; select only flat faces, or one sketch"
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

/// How the person chose to export, in the export dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ExportPlan {
    /// How finely a mesh follows the model (STL, 3MF, OBJ).
    pub quality: ExportQuality,
    /// Every body, whatever is selected: the way out the dialog offers
    /// when the selection does not fit the format.
    pub all_bodies: bool,
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
    /// Why the selection does not fit the format, said for the person.
    pub problem: Option<String>,
}

impl ExportCheck {
    /// Every body can be exported instead of the selection: a STEP or
    /// mesh file whose selection does not fit.
    pub fn offers_all_bodies(&self) -> bool {
        self.kind == Some(ExportKind::Bodies) && self.problem.is_some()
    }

    /// The export dialog is to be shown before the file is written: a
    /// mesh (its quality) or a selection that does not fit a STEP file.
    pub fn asks(&self) -> bool {
        self.mesh || self.offers_all_bodies()
    }
}

/// The text of an error without the command's name in front of it
/// (`file.export: a .step file …` → `A .step file …`), for a person.
pub fn plain_message(text: &str) -> String {
    let mut rest = text.trim();
    // `file.export: file.export: …` and the like, a few deep.
    for _ in 0..4 {
        match rest.split_once(": ") {
            Some((head, tail))
                if !head.is_empty()
                    && head.len() <= 24
                    && head
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_') =>
            {
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
    let problem = export_selection(session, name)
        .err()
        .map(|why| plain_message(&why));
    ExportCheck {
        kind,
        mesh: is_mesh(name),
        problem,
    }
}

/// The parameters `file.export` is given besides `path` for an export
/// to `name` as `plan` says: the selection's (unless every body was
/// chosen) and the quality of a mesh. An error when the selection does
/// not fit and every body was not chosen.
pub fn export_params(
    session: &mut Session,
    name: &str,
    plan: ExportPlan,
) -> Result<Vec<(&'static str, Value)>, String> {
    let mut params = Vec::new();
    let all = plan.all_bodies && export_kind(name) == Some(ExportKind::Bodies);
    if !all && let Some((key, value)) = export_selection(session, name)? {
        params.push((key, value));
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
    if check.mesh {
        ui.horizontal(|ui| {
            ui.label("Quality:");
            for quality in ExportQuality::ALL {
                ui.radio_value(&mut plan.quality, quality, quality.label())
                    .on_hover_text(quality.hint());
            }
        });
    }
    if let Some(problem) = &check.problem {
        ui.add(egui::Label::new(problem.as_str()).wrap());
    }
}

#[cfg(test)]
mod tests {
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
            all_bodies: false,
        };
        assert_eq!(
            export_params(&mut session, "a.stl", plan).unwrap(),
            vec![("quality", json!("fine"))]
        );
        assert!(
            export_params(&mut session, "a.step", plan)
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
        assert!(export_params(&mut session, "a.step", plan).is_err());
        let all = ExportPlan {
            all_bodies: true,
            ..plan
        };
        assert!(
            export_params(&mut session, "a.step", all)
                .unwrap()
                .is_empty()
        );
        // An outline cannot be every body.
        guarded(&mut session, "select.set", &json!({"mode": "clear"})).unwrap();
        let dxf = export_check(&mut session, "a.dxf");
        assert!(dxf.problem.is_some() && !dxf.offers_all_bodies() && !dxf.asks());
        assert!(export_params(&mut session, "a.dxf", all).is_err());
        // One body of two selected: those bodies, and the quality.
        guarded(
            &mut session,
            "select.set",
            &json!({"items": [{"kind": "body", "feature": "F2"}]}),
        )
        .unwrap();
        assert_eq!(
            export_params(&mut session, "a.3mf", plan).unwrap(),
            vec![("bodies", json!([2])), ("quality", json!("fine"))]
        );
    }

    #[test]
    fn names_are_proposed_from_the_document() {
        assert_eq!(suggested_name(None), "untitled.3dcraft");
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
