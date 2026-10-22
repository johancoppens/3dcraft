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

#[cfg(test)]
mod tests {
    use super::*;

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
