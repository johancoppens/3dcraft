//! The command registry: every action 3DCraft can take.
//!
//! To add a command: write its `fn(&mut Session, &Value) -> Result<Value,
//! CommandError>`, add a `CommandSpec` to `SPECS`, and add tests (behaviour,
//! undo, bad parameters). `tests/panic_hunt.rs` fuzzes every entry of
//! `SPECS` automatically.

use crate::params::Params;
use crate::{CommandError, CommandSpec, OpenDocument, Session, base64};
use craft3d_doc::{
    Block, Cylinder, DocError, Document, Expr, Feature, FeatureId, FeatureKind, LengthUnit, Sphere,
    Tolerance, Units,
};
#[cfg(not(target_arch = "wasm32"))]
use craft3d_io::MeshFormat;
use craft3d_math::Vec3;
use craft3d_regen::{Regen, Status};
use craft3d_render::{Camera, RenderParams};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_topo::{BodyKind, mass_properties};
use serde_json::{Value, json};

type Run = Result<Value, CommandError>;

fn fail<T>(message: impl Into<String>) -> Result<T, CommandError> {
    Err(CommandError::new(message))
}

fn doc_error(command: &str) -> impl Fn(DocError) -> CommandError + '_ {
    move |e| CommandError::new(format!("{command}: {e}"))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $summary:literal, doc: $needs:literal, $run:path) => {
        CommandSpec {
            id: $id,
            label: $label,
            params: $params,
            summary: $summary,
            needs_document: $needs,
            run: $run,
        }
    };
}

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("file.new", "New Document", r#"{"units": "um|mm|cm|m|in|ft" = "mm", "tolerance": number = 1e-6}"#,
        "Creates an empty document and makes it the active one.", doc: false, file_new),
    spec!("file.open", "Open", r#"{"path": string}"#,
        "Opens a .3dcraft file from the read root and makes it the active document.", doc: false, file_open),
    spec!("file.save", "Save", r#"{"path": string = the document's own path}"#,
        "Saves the active document as a .3dcraft file (atomically) under the write root.", doc: true, file_save),
    spec!("file.close", "Close", r#"{"discard": boolean = false}"#,
        "Closes the active document. Refuses unsaved changes unless discard is true.", doc: true, file_close),
    spec!("file.export", "Export Mesh", r#"{"path": string, "format": "stl|stl-ascii|3mf|obj" = from the extension, "tolerance": number = automatic}"#,
        "Tessellates every body and writes a mesh file under the write root. tolerance is the chordal tolerance in document units.", doc: true, file_export),
    spec!("edit.undo", "Undo", "{}", "Undoes the last change to the document.", doc: true, edit_undo),
    spec!("edit.redo", "Redo", "{}", "Redoes the change last undone.", doc: true, edit_redo),
    spec!("variable.set", "Set Variable", r#"{"name": string, "expr": expression}"#,
        "Defines a variable or replaces its expression, e.g. {\"name\": \"width\", \"expr\": \"40 mm\"}.", doc: true, variable_set),
    spec!("variable.delete", "Delete Variable", r#"{"name": string}"#,
        "Deletes a variable. Expressions that still use it fail at regeneration.", doc: true, variable_delete),
    spec!("variable.rename", "Rename Variable", r#"{"name": string, "to": string}"#,
        "Renames a variable and every reference to it in variables and features.", doc: true, variable_rename),
    spec!("feature.block", "Block", r#"{"size": [expr, expr, expr], "position": [expr, expr, expr] = [0, 0, 0], "name": string = automatic}"#,
        "Adds a rectangular block with one corner at position and edges along the world axes.", doc: true, feature_block),
    spec!("feature.cylinder", "Cylinder", r#"{"radius": expr, "height": expr, "position": [expr, expr, expr] = [0, 0, 0], "name": string = automatic}"#,
        "Adds a cylinder along the world z axis with its base centred at position.", doc: true, feature_cylinder),
    spec!("feature.sphere", "Sphere", r#"{"radius": expr, "position": [expr, expr, expr] = [0, 0, 0], "name": string = automatic}"#,
        "Adds a sphere centred at position.", doc: true, feature_sphere),
    spec!("feature.edit", "Edit Feature", r#"{"feature": id or name, "size": [expr; 3], "position": [expr; 3], "radius": expr, "height": expr}"#,
        "Changes parameters of a feature; give only the ones to change.", doc: true, feature_edit),
    spec!("feature.rename", "Rename Feature", r#"{"feature": id or name, "name": string}"#,
        "Renames a feature.", doc: true, feature_rename),
    spec!("feature.suppress", "Suppress Feature", r#"{"feature": id or name, "suppressed": boolean = true}"#,
        "Switches a feature off (or back on) without deleting it.", doc: true, feature_suppress),
    spec!("feature.move", "Reorder Feature", r#"{"feature": id or name, "position": integer}"#,
        "Moves a feature to a position in the history; 0 is first.", doc: true, feature_move),
    spec!("feature.delete", "Delete Feature", r#"{"feature": id or name}"#,
        "Deletes a feature.", doc: true, feature_delete),
    spec!("history.rollback", "Roll Back", r#"{"feature": id or name or null}"#,
        "Regenerates only up to and including this feature; null removes the marker.", doc: true, history_rollback),
    spec!("document.inspect", "Inspect", "{}",
        "Returns the whole document state: units, variables with values, features with status, bodies with counts and mass properties, undo history.", doc: true, document_inspect),
    spec!("document.measure", "Measure", r#"{"feature": id or name = all bodies}"#,
        "Returns volume, area and centroid, per body and in total, in document units.", doc: true, document_measure),
    spec!("view.render", "Render", r#"{"path": string, "width": integer = 800, "height": integer = 600, "view": "iso|front|back|left|right|top|bottom" = "iso", "mode": "shaded|ids" = "shaded", "tolerance": number = automatic}"#,
        "Renders the bodies with the CPU renderer. Writes a PNG under the write root when path is given, otherwise returns it as base64.", doc: true, view_render),
];

/// The command id most like `id`, for "did you mean".
pub(crate) fn closest(id: &str) -> Option<&'static str> {
    // Shared prefix length plus shared suffix length is a cheap similarity
    // that favours a typo inside an otherwise right id.
    let score = |candidate: &str| {
        let prefix = candidate
            .bytes()
            .zip(id.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let suffix = candidate
            .bytes()
            .rev()
            .zip(id.bytes().rev())
            .take_while(|(a, b)| a == b)
            .count();
        prefix + suffix
    };
    SPECS
        .iter()
        .map(|c| (score(c.id), c.id))
        .filter(|&(s, _)| s * 2 >= id.len().max(4))
        .max_by_key(|&(s, _)| s)
        .map(|(_, id)| id)
}

// ---------------------------------------------------------------- helpers

/// Applies an edit to the active document as one undo step.
fn edit<T>(
    session: &mut Session,
    command: &'static str,
    label: &str,
    change: impl FnOnce(&mut Document) -> Result<T, DocError>,
) -> Result<T, CommandError> {
    session
        .document_mut()?
        .history
        .apply(label, change)
        .map_err(doc_error(command))
}

/// Finds a feature by id (`3` or `"F3"`) or by name.
fn feature_ref(
    doc: &Document,
    p: &Params,
    command: &str,
    key: &str,
) -> Result<FeatureId, CommandError> {
    let value = p.get(key).ok_or_else(|| p.missing(key))?;
    let by_id = |n: u64| {
        doc.feature(FeatureId(n))
            .map(|f| f.id)
            .ok_or_else(|| CommandError::new(format!("{command}: there is no feature F{n}")))
    };
    match value {
        Value::Number(n) => match n.as_u64() {
            Some(n) => by_id(n),
            None => fail(format!(
                "{command}: `{key}` must be a feature id or name, got {n}"
            )),
        },
        Value::String(text) => {
            if let Some(feature) = doc.feature_by_name(text) {
                return Ok(feature.id);
            }
            match text
                .strip_prefix('F')
                .and_then(|digits| digits.parse::<u64>().ok())
            {
                Some(n) => by_id(n),
                None => {
                    let names: Vec<&str> = doc
                        .features
                        .iter()
                        .map(|f| f.name.as_str())
                        .take(20)
                        .collect();
                    fail(format!(
                        "{command}: there is no feature named `{text}`; features: {}",
                        if names.is_empty() {
                            "none".to_string()
                        } else {
                            names.join(", ")
                        }
                    ))
                }
            }
        }
        _ => fail(format!(
            "{command}: `{key}` must be a feature id (3 or \"F3\") or name"
        )),
    }
}

fn exprs(values: [String; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

fn origin() -> [String; 3] {
    ["0".to_string(), "0".to_string(), "0".to_string()]
}

fn feature_json(feature: &Feature) -> Value {
    json!({ "id": feature.id.to_string(), "name": feature.name })
}

fn add_feature(
    session: &mut Session,
    command: &'static str,
    name: Option<&str>,
    kind: FeatureKind,
) -> Run {
    let label = format!("Add {}", kind.type_name());
    let id = edit(session, command, &label, |doc| doc.add_feature(name, kind))?;
    feature_result(session, id)
}

/// What a feature command returns: the feature and how it regenerated, so
/// the caller sees at once whether the change worked.
fn feature_result(session: &mut Session, id: FeatureId) -> Run {
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let Some(feature) = doc.feature(id) else {
        return Ok(json!({ "id": id.to_string() }));
    };
    let mut result = feature_json(feature);
    if let (Some(map), Some(outcome)) = (result.as_object_mut(), regen.feature(id)) {
        let (status, error) = status_json(&outcome.status);
        map.insert("status".into(), status.into());
        if let Some(error) = error {
            map.insert("error".into(), error.into());
        }
    }
    Ok(result)
}

fn status_json(status: &Status) -> (&'static str, Option<&str>) {
    match status {
        Status::Ok => ("ok", None),
        Status::Suppressed => ("suppressed", None),
        Status::RolledBack => ("rolledBack", None),
        Status::Failed(reason) => ("failed", Some(reason)),
    }
}

fn vec3(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

fn length_unit(doc: &Document) -> &'static str {
    doc.units.length.symbol()
}

// ------------------------------------------------------------------- file

fn file_new(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.new", params, &["units", "tolerance"])?;
    let length = match p.str("units")? {
        None => LengthUnit::default(),
        Some(symbol) => LengthUnit::from_symbol(symbol).ok_or_else(|| {
            CommandError::new(format!(
                "file.new: unknown length unit `{symbol}`; use one of um, mm, cm, m, in, ft"
            ))
        })?,
    };
    let mut document = Document::new(Units {
        length,
        ..Units::default()
    });
    if let Some(linear) = p.number("tolerance")? {
        if linear <= 0.0 {
            return fail("file.new: `tolerance` must be positive");
        }
        document.tolerance = Tolerance {
            linear,
            ..Tolerance::default()
        };
    }
    let index = session.add_document(OpenDocument::new(document, None))?;
    Ok(json!({ "document": index, "units": length.symbol() }))
}

#[cfg(not(target_arch = "wasm32"))]
fn file_open(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.open", params, &["path"])?;
    let path = p.required_str("path")?;
    let resolved = session.workspace.resolve_read(path)?;
    let document = craft3d_format::load(&resolved)
        .map_err(|e| CommandError::new(format!("file.open: cannot open `{path}`: {e}")))?;
    let features = document.features.len();
    let index = session.add_document(OpenDocument::new(document, Some(path.to_string())))?;
    Ok(json!({ "document": index, "path": path, "features": features }))
}

#[cfg(not(target_arch = "wasm32"))]
fn file_save(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.save", params, &["path"])?;
    let path = match (p.str("path")?, &session.document()?.path) {
        (Some(path), _) => path.to_string(),
        (None, Some(own)) => own.clone(),
        (None, None) => return fail("file.save: this document has no file yet; give a `path`"),
    };
    let resolved = session.workspace.resolve_write(&path)?;
    let open = session.document_mut()?;
    craft3d_format::save(&resolved, open.history.current())
        .map_err(|e| CommandError::new(format!("file.save: cannot save `{path}`: {e}")))?;
    open.saved_revision = Some(open.history.revision());
    open.path = Some(path.clone());
    Ok(json!({ "path": path }))
}

#[cfg(target_arch = "wasm32")]
fn file_open(_: &mut Session, _: &Value) -> Run {
    fail("file.open: files are not available in this build")
}

#[cfg(target_arch = "wasm32")]
fn file_save(_: &mut Session, _: &Value) -> Run {
    fail("file.save: files are not available in this build")
}

fn file_close(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.close", params, &["discard"])?;
    let discard = p.bool("discard")?.unwrap_or(false);
    let open = session.document()?;
    if open.dirty()
        && !discard
        && (open.path.is_some() || !open.history.current().features.is_empty())
    {
        return fail(
            "file.close: the document has unsaved changes; save it, or pass {\"discard\": true}",
        );
    }
    session.documents.remove(session.active);
    session.active = session.documents.len().saturating_sub(1);
    Ok(json!({ "open": session.documents.len() }))
}

/// A default chordal tolerance that suits the size of the model: a
/// thousandth of the side of a square with the bodies' total area.
fn automatic_tolerance(regen: &Regen) -> f64 {
    let area: f64 = regen
        .bodies()
        .filter_map(|(_, body)| mass_properties(body).ok())
        .map(|m| m.area)
        .sum();
    if area.is_finite() && area > 0.0 {
        area.sqrt() * 1e-3
    } else {
        1e-3
    }
}

fn tolerance_param(p: &Params, command: &str, regen: &Regen) -> Result<TessParams, CommandError> {
    let chordal_tolerance = match p.number("tolerance")? {
        Some(t) if t > 0.0 => t,
        Some(_) => return fail(format!("{command}: `tolerance` must be positive")),
        None => automatic_tolerance(regen),
    };
    Ok(TessParams {
        chordal_tolerance,
        ..TessParams::default()
    })
}

/// One mesh per body, with the name of the feature that made it.
fn meshes(
    open: &mut OpenDocument,
    command: &str,
    tess: &TessParams,
) -> Result<Vec<(String, Mesh)>, CommandError> {
    let regen = open.regen();
    let doc = open.history.current();
    let mut out = Vec::new();
    for (id, body) in regen.bodies() {
        let name = doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        let mesh = tessellate_body(body, tess)
            .map_err(|e| CommandError::new(format!("{command}: {name}: {e}")))?;
        out.push((name, mesh));
    }
    if out.is_empty() {
        return fail(format!(
            "{command}: there are no bodies; add a feature, or check document.inspect for failed features"
        ));
    }
    Ok(out)
}

#[cfg(not(target_arch = "wasm32"))]
fn file_export(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.export", params, &["path", "format", "tolerance"])?;
    let path = p.required_str("path")?;
    let format = match p.str("format")? {
        Some("stl") => MeshFormat::StlBinary,
        Some("stl-ascii") => MeshFormat::StlAscii,
        Some("3mf") => MeshFormat::ThreeMf,
        Some("obj") => MeshFormat::Obj,
        Some(other) => return fail(format!("file.export: unknown format `{other}`; use stl, stl-ascii, 3mf or obj")),
        None => std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(MeshFormat::from_extension)
            .ok_or_else(|| {
                CommandError::new(format!(
                    "file.export: cannot tell the format from `{path}`; use a .stl, .3mf or .obj name, or pass `format`"
                ))
            })?,
    };
    let resolved = session.workspace.resolve_write(path)?;
    let open = session.document_mut()?;
    let tess = tolerance_param(&p, "file.export", &open.regen())?;
    let meshes = meshes(open, "file.export", &tess)?;
    let unit = match open.history.current().units.length {
        LengthUnit::Micrometre => craft3d_meshio::Unit::Micron,
        LengthUnit::Millimetre => craft3d_meshio::Unit::Millimeter,
        LengthUnit::Centimetre => craft3d_meshio::Unit::Centimeter,
        LengthUnit::Metre => craft3d_meshio::Unit::Meter,
        LengthUnit::Inch => craft3d_meshio::Unit::Inch,
        LengthUnit::Foot => craft3d_meshio::Unit::Foot,
    };
    let mut model = craft3d_meshio::Model {
        unit,
        objects: Vec::new(),
    };
    let mut triangles = 0;
    for (name, mesh) in &meshes {
        let object = craft3d_io::to_object(mesh, name)
            .map_err(|e| CommandError::new(format!("file.export: {name}: {e}")))?;
        triangles += object.triangles.len();
        model.objects.push(object);
    }
    let bytes = craft3d_io::encode(&model, format)
        .map_err(|e| CommandError::new(format!("file.export: {e}")))?;
    crate::workspace::write_atomic(&resolved, &bytes)?;
    Ok(json!({
        "path": path,
        "bytes": bytes.len(),
        "bodies": model.objects.len(),
        "triangles": triangles,
        "tolerance": tess.chordal_tolerance,
        "unit": length_unit(open.history.current()),
    }))
}

#[cfg(target_arch = "wasm32")]
fn file_export(_: &mut Session, _: &Value) -> Run {
    fail("file.export: files are not available in this build")
}

// ------------------------------------------------------------------- edit

fn edit_undo(session: &mut Session, params: &Value) -> Run {
    Params::new("edit.undo", params, &[])?;
    match session.document_mut()?.history.undo() {
        Some(label) => Ok(json!({ "undone": label })),
        None => fail("edit.undo: there is nothing to undo"),
    }
}

fn edit_redo(session: &mut Session, params: &Value) -> Run {
    Params::new("edit.redo", params, &[])?;
    match session.document_mut()?.history.redo() {
        Some(label) => Ok(json!({ "redone": label })),
        None => fail("edit.redo: there is nothing to redo"),
    }
}

// -------------------------------------------------------------- variables

/// A variable with its value, as commands return it.
fn variable_json(regen: &Regen, name: &str, unit: &str) -> Value {
    match regen.variables.iter().find(|v| v.name == name) {
        None => json!({ "name": name }),
        Some(v) => match &v.value {
            Ok(value) => json!({
                "name": v.name, "expr": v.expr, "value": value.value,
                "dimension": value.dimension.to_string(), "lengthUnit": unit,
            }),
            Err(error) => json!({ "name": v.name, "expr": v.expr, "error": error }),
        },
    }
}

fn variable_set(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("variable.set", params, &["name", "expr"])?;
    let name = p.required_str("name")?;
    let expr = p.expr("expr")?.ok_or_else(|| p.missing("expr"))?;
    edit(session, "variable.set", &format!("Set {name}"), |doc| {
        doc.set_variable(name, &expr)
    })?;
    let open = session.document_mut()?;
    let regen = open.regen();
    Ok(variable_json(
        &regen,
        name,
        length_unit(open.history.current()),
    ))
}

fn variable_delete(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("variable.delete", params, &["name"])?;
    let name = p.required_str("name")?;
    edit(
        session,
        "variable.delete",
        &format!("Delete {name}"),
        |doc| doc.remove_variable(name),
    )?;
    Ok(json!({ "deleted": name }))
}

fn variable_rename(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("variable.rename", params, &["name", "to"])?;
    let (name, to) = (p.required_str("name")?, p.required_str("to")?);
    edit(
        session,
        "variable.rename",
        &format!("Rename {name}"),
        |doc| doc.rename_variable(name, to),
    )?;
    Ok(json!({ "name": to }))
}

// --------------------------------------------------------------- features

fn feature_block(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.block", params, &["size", "position", "name"])?;
    let size = p.expr3("size")?.ok_or_else(|| p.missing("size"))?;
    let kind = FeatureKind::Block(Block {
        position: exprs(p.expr3("position")?.unwrap_or_else(origin)),
        size: exprs(size),
    });
    add_feature(session, "feature.block", p.str("name")?, kind)
}

fn feature_cylinder(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.cylinder",
        params,
        &["radius", "height", "position", "name"],
    )?;
    let kind = FeatureKind::Cylinder(Cylinder {
        position: exprs(p.expr3("position")?.unwrap_or_else(origin)),
        radius: Expr::new(p.expr("radius")?.ok_or_else(|| p.missing("radius"))?),
        height: Expr::new(p.expr("height")?.ok_or_else(|| p.missing("height"))?),
    });
    add_feature(session, "feature.cylinder", p.str("name")?, kind)
}

fn feature_sphere(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.sphere", params, &["radius", "position", "name"])?;
    let kind = FeatureKind::Sphere(Sphere {
        position: exprs(p.expr3("position")?.unwrap_or_else(origin)),
        radius: Expr::new(p.expr("radius")?.ok_or_else(|| p.missing("radius"))?),
    });
    add_feature(session, "feature.sphere", p.str("name")?, kind)
}

fn feature_edit(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.edit",
        params,
        &["feature", "size", "position", "radius", "height"],
    )?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.edit",
        "feature",
    )?;
    let (size, position) = (p.expr3("size")?, p.expr3("position")?);
    let (radius, height) = (p.expr("radius")?, p.expr("height")?);
    if size.is_none() && position.is_none() && radius.is_none() && height.is_none() {
        return fail(
            "feature.edit: give at least one parameter to change (size, position, radius, height)",
        );
    }
    let mut kind = session
        .document()?
        .history
        .current()
        .feature(id)
        .map(|f| f.kind.clone())
        .ok_or_else(|| CommandError::new(format!("feature.edit: there is no feature {id}")))?;
    // Each kind takes what applies to it; anything left over is an error.
    let (mut size, mut position, mut radius, mut height) = (size, position, radius, height);
    let (allowed, name) = match &mut kind {
        FeatureKind::Block(b) => {
            if let Some(v) = size.take() {
                b.size = exprs(v);
            }
            if let Some(v) = position.take() {
                b.position = exprs(v);
            }
            ("position, size", "block")
        }
        FeatureKind::Cylinder(c) => {
            if let Some(v) = position.take() {
                c.position = exprs(v);
            }
            if let Some(v) = radius.take() {
                c.radius = Expr::new(v);
            }
            if let Some(v) = height.take() {
                c.height = Expr::new(v);
            }
            ("position, radius, height", "cylinder")
        }
        FeatureKind::Sphere(s) => {
            if let Some(v) = position.take() {
                s.position = exprs(v);
            }
            if let Some(v) = radius.take() {
                s.radius = Expr::new(v);
            }
            ("position, radius", "sphere")
        }
    };
    let leftover = [
        ("size", size.is_some()),
        ("position", position.is_some()),
        ("radius", radius.is_some()),
        ("height", height.is_some()),
    ];
    if let Some((unused, _)) = leftover.iter().find(|(_, left)| *left) {
        return fail(format!(
            "feature.edit: `{unused}` is not a parameter of a {name}; its parameters are: {allowed}"
        ));
    }
    edit(session, "feature.edit", "Edit feature", |doc| {
        doc.feature_mut(id)?.kind = kind;
        Ok(())
    })?;
    feature_result(session, id)
}

fn feature_rename(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.rename", params, &["feature", "name"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.rename",
        "feature",
    )?;
    let name = p.required_str("name")?;
    edit(session, "feature.rename", "Rename feature", |doc| {
        doc.rename_feature(id, name)
    })?;
    feature_result(session, id)
}

fn feature_suppress(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.suppress", params, &["feature", "suppressed"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.suppress",
        "feature",
    )?;
    let suppressed = p.bool("suppressed")?.unwrap_or(true);
    let label = if suppressed {
        "Suppress feature"
    } else {
        "Unsuppress feature"
    };
    edit(session, "feature.suppress", label, |doc| {
        doc.feature_mut(id)?.suppressed = suppressed;
        Ok(())
    })?;
    feature_result(session, id)
}

fn feature_move(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.move", params, &["feature", "position"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.move",
        "feature",
    )?;
    let position = p
        .integer("position", 0, u64::from(u32::MAX))?
        .ok_or_else(|| p.missing("position"))?;
    // Bounded by the check above.
    edit(session, "feature.move", "Reorder feature", |doc| {
        doc.move_feature(id, position as usize)
    })?;
    feature_result(session, id)
}

fn feature_delete(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.delete", params, &["feature"])?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.delete",
        "feature",
    )?;
    let removed = edit(session, "feature.delete", "Delete feature", |doc| {
        doc.remove_feature(id)
    })?;
    Ok(json!({ "deleted": feature_json(&removed) }))
}

fn history_rollback(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("history.rollback", params, &["feature"])?;
    if !p.has("feature") {
        return Err(p.missing("feature"));
    }
    let target = match p.get("feature") {
        None => None,
        Some(_) => Some(feature_ref(
            session.document()?.history.current(),
            &p,
            "history.rollback",
            "feature",
        )?),
    };
    edit(session, "history.rollback", "Roll back", |doc| {
        doc.set_rollback(target)
    })?;
    Ok(json!({ "rollback": target.map(|id| id.to_string()) }))
}

// --------------------------------------------------------------- document

fn document_inspect(session: &mut Session, params: &Value) -> Run {
    Params::new("document.inspect", params, &[])?;
    let index = session.active;
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let unit = length_unit(doc);

    let variables: Vec<Value> = regen
        .variables
        .iter()
        .map(|v| variable_json(&regen, &v.name, unit))
        .collect();
    let mut features = Vec::new();
    let mut bodies = Vec::new();
    for feature in &doc.features {
        let outcome = regen.feature(feature.id);
        let (status, error) = outcome.map_or(("unknown", None), |o| status_json(&o.status));
        let mut entry = json!({
            "id": feature.id.to_string(),
            "name": feature.name,
            "suppressed": feature.suppressed,
            "status": status,
            "params": serde_json::to_value(&feature.kind).unwrap_or(Value::Null),
        });
        if let (Some(map), Some(error)) = (entry.as_object_mut(), error) {
            map.insert("error".into(), error.into());
        }
        features.push(entry);
        if let Some(body) = outcome.and_then(|o| o.body.as_ref()) {
            let mut entry = json!({
                "feature": feature.id.to_string(),
                "name": feature.name,
                "kind": match body.kind { BodyKind::Solid => "solid", BodyKind::Sheet => "sheet" },
                "valid": true,
                "faces": body.faces.len(),
                "edges": body.edges.len(),
                "vertices": body.vertices.len(),
            });
            if let (Some(map), Ok(mass)) = (entry.as_object_mut(), mass_properties(body)) {
                map.insert("volume".into(), mass.volume.into());
                map.insert("area".into(), mass.area.into());
                map.insert("centroid".into(), mass.centroid.map_or(Value::Null, vec3));
            }
            bodies.push(entry);
        }
    }
    Ok(json!({
        "document": index,
        "path": open.path,
        "dirty": open.dirty(),
        "units": { "length": unit, "angle": match doc.units.angle {
            craft3d_doc::AngleUnit::Degree => "deg",
            craft3d_doc::AngleUnit::Radian => "rad",
        }},
        "tolerance": { "linear": doc.tolerance.linear, "angular": doc.tolerance.angular },
        "variables": variables,
        "features": features,
        "rollback": doc.rollback.map(|id| id.to_string()),
        "bodies": bodies,
        "ok": regen.is_ok(),
        "history": {
            "undo": open.history.undo_labels().collect::<Vec<_>>(),
            "redo": open.history.redo_labels().collect::<Vec<_>>(),
        },
    }))
}

fn document_measure(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("document.measure", params, &["feature"])?;
    let only = match p.get("feature") {
        None => None,
        Some(_) => Some(feature_ref(
            session.document()?.history.current(),
            &p,
            "document.measure",
            "feature",
        )?),
    };
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let unit = length_unit(doc);
    let mut bodies = Vec::new();
    let (mut volume, mut area, mut moment) = (0.0, 0.0, Vec3::ZERO);
    for (id, body) in regen
        .bodies()
        .filter(|(id, _)| only.is_none_or(|o| o == *id))
    {
        let name = doc
            .feature(id)
            .map_or_else(|| id.to_string(), |f| f.name.clone());
        let mass = mass_properties(body)
            .map_err(|e| CommandError::new(format!("document.measure: {name}: {e}")))?;
        volume += mass.volume;
        area += mass.area;
        if let Some(c) = mass.centroid {
            moment += c * mass.volume;
        }
        bodies.push(json!({
            "feature": id.to_string(), "name": name,
            "volume": mass.volume, "area": mass.area, "centroid": mass.centroid.map_or(Value::Null, vec3),
        }));
    }
    if let (Some(id), true) = (only, bodies.is_empty()) {
        let status = regen
            .feature(id)
            .map_or("unknown", |o| status_json(&o.status).0);
        return fail(format!(
            "document.measure: feature {id} has no body (its status is {status})"
        ));
    }
    Ok(json!({
        "bodies": bodies,
        "volume": volume,
        "area": area,
        // Bodies are summed as they are: overlapping bodies count twice.
        "centroid": if volume != 0.0 { vec3(moment * (1.0 / volume)) } else { Value::Null },
        "lengthUnit": unit,
        "areaUnit": format!("{unit}^2"),
        "volumeUnit": format!("{unit}^3"),
    }))
}

// ------------------------------------------------------------------- view

fn view_render(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "view.render",
        params,
        &["path", "width", "height", "view", "mode", "tolerance"],
    )?;
    let width = p.integer("width", 16, 4096)?.unwrap_or(800);
    let height = p.integer("height", 16, 4096)?.unwrap_or(600);
    let view = p.str("view")?.unwrap_or("iso");
    let (toward_eye, up) = match view {
        "iso" => (Vec3::new(1.0, -1.0, 0.8), Vec3::Z),
        "front" => (Vec3::new(0.0, -1.0, 0.0), Vec3::Z),
        "back" => (Vec3::new(0.0, 1.0, 0.0), Vec3::Z),
        "right" => (Vec3::new(1.0, 0.0, 0.0), Vec3::Z),
        "left" => (Vec3::new(-1.0, 0.0, 0.0), Vec3::Z),
        "top" => (Vec3::new(0.0, 0.0, 1.0), Vec3::Y),
        "bottom" => (Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
        other => {
            return fail(format!(
                "view.render: unknown view `{other}`; use iso, front, back, left, right, top or bottom"
            ));
        }
    };
    let ids = match p.str("mode")?.unwrap_or("shaded") {
        "shaded" => false,
        "ids" => true,
        other => {
            return fail(format!(
                "view.render: unknown mode `{other}`; use shaded or ids"
            ));
        }
    };
    let path = p.str("path")?;
    let resolved = path
        .map(|path| session.workspace.resolve_write(path))
        .transpose()?;

    let open = session.document_mut()?;
    let tess = tolerance_param(&p, "view.render", &open.regen())?;
    let meshes = meshes(open, "view.render", &tess)?;
    // One mesh for the renderer; face ids keep counting across bodies.
    let mut all = Mesh::default();
    let mut legend = Vec::new();
    for (name, mesh) in &meshes {
        let vertex_base = u32::try_from(all.positions.len())
            .map_err(|_| CommandError::new("view.render: mesh too large"))?;
        let face_base = all.face_ids.iter().copied().max().map_or(0, |m| m + 1);
        all.positions.extend(&mesh.positions);
        all.normals.extend(&mesh.normals);
        all.triangles
            .extend(mesh.triangles.iter().map(|t| t.map(|k| k + vertex_base)));
        all.face_ids
            .extend(mesh.face_ids.iter().map(|f| f + face_base));
        legend.push(json!({ "body": name, "firstFaceId": face_base }));
    }
    // Both are within 16..=4096.
    let render_params = RenderParams {
        width: width as u32,
        height: height as u32,
        ..RenderParams::default()
    };
    let aspect = width as f64 / height as f64;
    let camera = Camera::framing(&all.bbox(), toward_eye, up, 30.0, aspect)
        .ok_or_else(|| CommandError::new("view.render: cannot frame the model"))?;
    let frame = craft3d_render::render(&all, &camera, &render_params)
        .map_err(|e| CommandError::new(format!("view.render: {e}")))?;
    let png = if ids {
        craft3d_render::encode_png(frame.width, frame.height, &frame.false_color())
    } else {
        frame.to_png()
    }
    .map_err(|e| CommandError::new(format!("view.render: {e}")))?;

    let mut result = json!({
        "width": width, "height": height, "view": view,
        "triangles": all.triangles.len(),
        "coverage": frame.coverage(),
        "bodies": legend,
    });
    let Some(map) = result.as_object_mut() else {
        return Ok(result);
    };
    match (path, resolved) {
        #[cfg(not(target_arch = "wasm32"))]
        (Some(path), Some(resolved)) => {
            crate::workspace::write_atomic(&resolved, &png)?;
            map.insert("path".into(), path.into());
        }
        _ => {
            map.insert("mime".into(), "image/png".into());
            map.insert("base64".into(), base64::encode(&png).into());
        }
    }
    Ok(result)
}
