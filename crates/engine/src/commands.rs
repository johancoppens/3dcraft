//! The command registry: every action 3DCraft can take.
//!
//! To add a command: write its `fn(&mut Session, &Value) -> Result<Value,
//! CommandError>`, add a `CommandSpec` to `SPECS`, and add tests (behaviour,
//! undo, bad parameters). `tests/panic_hunt.rs` fuzzes every entry of
//! `SPECS` automatically.

use crate::mass::mass_of;
use crate::params::Params;
use crate::{CommandError, CommandSpec, OpenDocument, Session, base64};
use craft3d_doc::{
    Block, Boolean, BooleanOp, Cone, Cylinder, DocError, Document, Expr, Extrude, ExtrudeEnd,
    Feature, FeatureId, FeatureKind, LengthUnit, Orientation, PatternLayout, PersistentName,
    PlaneBy, Sphere, Tolerance, Torus, Units,
};
#[cfg(not(target_arch = "wasm32"))]
use craft3d_io::MeshFormat;
use craft3d_math::Vec3;
use craft3d_regen::{Regen, Status};
use craft3d_render::{Camera, RenderParams};
use craft3d_tess::{Mesh, TessParams, tessellate_body};
use craft3d_topo::BodyKind;
use serde_json::{Value, json};

pub(crate) type Run = Result<Value, CommandError>;

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T, CommandError> {
    Err(CommandError::new(message))
}

pub(crate) fn doc_error(command: &str) -> impl Fn(DocError) -> CommandError + '_ {
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
pub(crate) use spec;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("file.new", "New Document", r#"{"units": "um|mm|cm|m|in|ft" = "mm", "tolerance": number = 1e-6}"#,
        "Creates an empty document and makes it the active one.", doc: false, file_new),
    spec!("file.open", "Open", r#"{"path": string}"#,
        "Opens a .3dcraft file from the read root and makes it the active document.", doc: false, file_open),
    spec!("file.save", "Save", r#"{"path": string = the document's own path, "overwrite": boolean = false}"#,
        "Saves the active document as a .3dcraft file (atomically) under the write root. A recovered document (recovery.open) is not saved over its original file when that file changed since the recovery was taken, unless overwrite is true.", doc: true, file_save),
    spec!("file.close", "Close", r#"{"discard": boolean = false}"#,
        "Closes the active document. Refuses unsaved changes unless discard is true.", doc: true, file_close),
    spec!("file.export", "Export", r#"{"path": string, "format": "step|stl|stl-ascii|3mf|obj|dxf|svg" = from the extension, "tolerance": number = automatic, "sketch": id or name, "face": face name, "feature": id or name = the body that has the face, "construction": boolean = false}"#,
        "Writes a file under the write root. step (.step, .stp) is the exact geometry of every body, with the document's units, its tolerance as the file's uncertainty and the bodies' names. stl, 3mf and obj are meshes of every body: tolerance is the chordal tolerance in document units, and closed counts the meshes that are watertight. dxf and svg are a flat outline for laser cutting and drawings, in the document's unit at true size: of a sketch (sketch: its lines, arcs and circles joined into chains, in the sketch's axes; construction geometry only with construction, on a layer of its own) or of a flat face of a body (face: a name from document.inspect, faceNames; its outer boundary and its holes, seen from outside the body; feature says which body when several have a face of that name). Every curve stays what it is: lines, circular arcs and circles, ellipses, splines; nothing becomes a polyline. dxf is AutoCAD 2000 ASCII with $INSUNITS; svg has width and height in mm, cm or in and y pointing up as in the model. tolerance for svg is how far the cubic Béziers written for a spline of a degree above 3 or a rational one may lie from it (default: the document's tolerance); the result says how many there were.", doc: true, file_export),
    spec!("edit.undo", "Undo", "{}", "Undoes the last change to the document.", doc: true, edit_undo),
    spec!("edit.redo", "Redo", "{}", "Redoes the change last undone.", doc: true, edit_redo),
    spec!("variable.set", "Set Variable", r#"{"name": string, "expr": expression}"#,
        "Defines a variable or replaces its expression, e.g. {\"name\": \"width\", \"expr\": \"40 mm\"}.", doc: true, variable_set),
    spec!("variable.delete", "Delete Variable", r#"{"name": string}"#,
        "Deletes a variable. Expressions that still use it fail at regeneration.", doc: true, variable_delete),
    spec!("variable.rename", "Rename Variable", r#"{"name": string, "to": string}"#,
        "Renames a variable and every reference to it in variables and features.", doc: true, variable_rename),
    spec!("feature.block", "Block", r#"{"size": [expr, expr, expr], "position": [expr, expr, expr] = [0, 0, 0], "axis": [number, number, number] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic}"#,
        "Adds a rectangular block with one corner at position. axis is the direction of its local z, angle a turn about that axis.", doc: true, feature_block),
    spec!("feature.cylinder", "Cylinder", r#"{"radius": expr, "height": expr, "position": [expr, expr, expr] = [0, 0, 0], "axis": [number, number, number] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic}"#,
        "Adds a cylinder along axis with its base centred at position.", doc: true, feature_cylinder),
    spec!("feature.sphere", "Sphere", r#"{"radius": expr, "position": [expr, expr, expr] = [0, 0, 0], "name": string = automatic}"#,
        "Adds a sphere centred at position.", doc: true, feature_sphere),
    spec!("feature.cone", "Cone", r#"{"radius": expr, "height": expr, "position": [expr, expr, expr] = [0, 0, 0], "axis": [number, number, number] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic}"#,
        "Adds a cone along axis: its base circle is centred at position, its apex is height further along the axis.", doc: true, feature_cone),
    spec!("feature.torus", "Torus", r#"{"major": expr, "minor": expr, "position": [expr, expr, expr] = [0, 0, 0], "axis": [number, number, number] = [0, 0, 1], "angle": expr = "0 deg", "name": string = automatic}"#,
        "Adds a ring torus about axis, centred at position: major is the radius of the ring, minor of the tube.", doc: true, feature_torus),
    spec!("feature.extrude", "Extrude", r#"{"sketch": id or name, "distance": expr, "throughAll": boolean = false, "upTo": face name, "symmetric": boolean = false, "reversed": boolean = false, "cut": id or name, "join": id or name, "intersect": id or name, "name": string = automatic}"#,
        "Adds a solid by sweeping the closed profiles of a sketch along its normal: loops of lines and arcs, and circles; a loop inside another is a hole. Give one end: distance (a negative one goes the other way; symmetric puts half on each side of the sketch plane), throughAll (past every body the features before this one have built that lies in the way; symmetric goes through on both sides) or upTo (a face name from document.inspect: the solid ends on that face's surface, flat or curved, and follows it when the face moves). reversed goes against the sketch's normal. With cut, join or intersect (a feature with a body) the solid is combined with that body at once: two features are added in one undo step, the extrusion and a boolean, and the result is the boolean's (the one to build on; name applies to it) with the extrusion under sweep. A hole through a plate: sketch a circle on its top, extrude with throughAll, reversed and cut.", doc: true, feature_extrude),
    spec!("feature.edit", "Edit Feature", r#"{"feature": id or name, "sketch": id or name, "path": id or name, "sections": [sketch, or {sketch, start}; ...], "ruled": boolean, "closed": boolean, "source": id or name, "target": id or name, "tool": id or name, "op": "union|subtract|intersect", "size": [expr; 3] or a thread's "M10" or expr or null, "position": [expr; 3], "axis": [number; 3] or datum axis or "x|y" or line id, "angle": expr, "radius": expr, "height": expr, "major": expr, "minor": expr, "distance": expr, "throughAll": boolean, "upTo": face name, "symmetric": boolean, "reversed": boolean, "translate": [expr; 3], "pivot": [expr; 3], "point": [expr; 3], "normal": [number; 3], "plane": datum plane id or name, "step": [expr; 3], "count": expr, "keep": boolean, "keepTool": boolean, "edges": [[face name, face name], ...], "second": expr, "chain": boolean, "faces": [face name, ...], "thickness": expr, "open": [face name, ...], "pitch": expr (or null for a thread: the coarse pitch of its size), "turns": expr, "leftHanded": boolean, "length": expr, "cosmetic": boolean, "modeled": boolean}"#,
        "Changes parameters of a feature; give only the ones to change. Which ones a feature has depends on its kind (document.inspect shows them under params). The end of an extrusion is changed by giving the new one: distance, throughAll or upTo. A mirror is pointed at a datum plane with plane, and back at a plane of its own with normal or point; a circular pattern at a datum axis with axis (its name or id), and back at a line of its own with axis as a direction or point. What a feature is made from is changed the same way, which is how a feature that lost it is repaired: sketch for an extrusion, a revolution or a sweep (its profile), path for a sweep, axis for a revolution (x, y, a line of the sketch or a datum axis), source for a moved, mirrored or patterned body, target, tool and op for a boolean, edges for a chamfer or a fillet (which is also how its body is chosen: the body of the feature the faces are of); the feature keeps its id and its place in the history, and the names of its faces are those of a feature built that way from the start. A chamfer's size: distance alone changes the first distance and keeps the form; with second it becomes two distances, with angle a distance and an angle (give second the same as distance for equal ones). A fillet's radius is changed with radius. An offset's faces and distance are changed with faces and distance (the faces also choose its body, as the edges of a chamfer do), a deletion's faces with faces; a shell's with source, thickness and open. A datum plane's distance or angle and the position of a datum point at coordinates are changed here; what a datum is placed by, with datum.plane, datum.axis or datum.point and replace.", doc: true, feature_edit),
    spec!("feature.rename", "Rename Feature", r#"{"feature": id or name, "name": string}"#,
        "Renames a feature.", doc: true, feature_rename),
    spec!("feature.suppress", "Suppress Feature", r#"{"feature": id or name, "suppressed": boolean = true}"#,
        "Switches a feature off (or back on) without deleting it.", doc: true, feature_suppress),
    spec!("feature.move", "Reorder Feature", r#"{"feature": id or name, "position": integer}"#,
        "Moves a feature to a position in the history; 0 is first.", doc: true, feature_move),
    spec!("feature.delete", "Delete Feature", r#"{"feature": id or name}"#,
        "Deletes a feature.", doc: true, feature_delete),
    spec!("history.rollback", "Roll Back", r#"{"feature": id or name or null}"#,
        "Regenerates only up to and including this feature; null removes the marker. While the marker is set, a feature added by any command goes in right after it, and the marker moves onto the new feature.", doc: true, history_rollback),
    spec!("document.inspect", "Inspect", "{}",
        "Returns the whole document state: units, variables with values, features with status (a failed one with the reason under error, which starts with the parameter and names the feature and the name it lost, and, when the reason is that something it is made from failed, under causedBy the feature at the start of that chain: the one to fix; a datum with where it lies, under datum), bodies with counts, mass properties and faceNames (and, per face, faceAliases: the other names a face answers to when a boolean or a pattern merged faces of several operands into it), undo history.", doc: true, document_inspect),
    spec!("document.check", "Check", r#"{"deep": boolean = false}"#,
        "Checks every body for validity and lists every problem: failed variables, failed features (with causedBy as in document.inspect), defects of bodies. ok is true when there is none. deep also looks for bodies that pass through themselves (faces crossing, shells overlapping or nested the wrong way), on a fine mesh: slower, and what it reports is certain (deeper than the mesh could be off); doubts are counted under marginal.", doc: true, document_check),
    spec!("document.measure", "Measure", r#"{"feature": id or name = all bodies}"#,
        "Returns volume, area and centroid, per body and in total, in document units.", doc: true, document_measure),
    spec!("view.render", "Render", r#"{"path": string, "width": integer = 800, "height": integer = 600, "view": "iso|front|back|left|right|top|bottom" = "iso", "projection": "perspective|orthographic" = "perspective", "zoom": number = 1, "box": [[x, y, z], [x, y, z]] = the whole model, "mode": "shaded|ids" = "shaded", "section": {plane, feature, point, normal, offset, flip, cap} = none, "tolerance": number = automatic}"#,
        "Renders the bodies with the CPU renderer. Writes a PNG under the write root when path is given, otherwise returns it as base64. orthographic shows lengths across the view to scale. box frames the region between two opposite corners (document units) instead of the whole model, seen from outside the model. zoom magnifies about the centre of the picture (2 shows half as much, twice as large; from 0.01 to 1000). section cuts the picture open with a plane, to look inside a shell, a pocket or a cavity; the document does not change. It is an object: the plane is plane (xy, yz or zx, a datum plane by id or name, or a flat face by its persistent name, with feature to say which body when several have that face) or point and normal ([x, y, z] each), moved by offset (a number, default 0) along its normal; what lies on the side the normal points to is not drawn (a face's normal points out of its body; flip: true keeps the other half). Where the plane passes through the inside of a body the cut is closed with a flat cap in a colour of its own (cap: false leaves it open). The result then has section: {point, normal, range}, the plane as it was used and the offsets between which it passes through the model.", doc: true, view_render),
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
    Session::commands()
        .iter()
        .map(|c| (score(c.id), c.id))
        .filter(|&(s, _)| s * 2 >= id.len().max(4))
        .max_by_key(|&(s, _)| s)
        .map(|(_, id)| id)
}

// ---------------------------------------------------------------- helpers

/// Applies an edit to the active document as one undo step.
pub(crate) fn edit<T>(
    session: &mut Session,
    command: &'static str,
    label: &str,
    change: impl FnOnce(&mut Document) -> Result<T, DocError>,
) -> Result<T, CommandError> {
    // Every edit ends with the sketches solved and their solution stored,
    // in the same undo step: a changed variable moves the sketches that
    // use it, and the next solve starts from where they are now.
    let open = session.document_mut()?;
    let cache = &mut open.cache;
    open.history
        .apply(label, |doc| {
            let out = change(doc)?;
            craft3d_regen::settle_sketches_cached(doc, cache);
            Ok(out)
        })
        .map_err(doc_error(command))
}

/// Finds a feature by id (`3` or `"F3"`) or by name.
pub(crate) fn feature_ref(
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

pub(crate) fn exprs(values: [String; 3]) -> [Expr; 3] {
    values.map(Expr::new)
}

pub(crate) fn origin() -> [String; 3] {
    ["0".to_string(), "0".to_string(), "0".to_string()]
}

fn feature_json(feature: &Feature) -> Value {
    json!({ "id": feature.id.to_string(), "name": feature.name })
}

/// Adds a feature to the history where a modeller does: at the end, or,
/// with the history rolled back, right after the rollback marker, which
/// then moves onto the new feature. What came after the marker stays
/// rolled back, after the new feature. Every command that adds a feature
/// goes through here.
pub(crate) fn insert_feature(
    doc: &mut Document,
    name: Option<&str>,
    kind: FeatureKind,
) -> Result<FeatureId, DocError> {
    let id = doc.add_feature(name, kind)?;
    if let Some(marker) = doc.rollback {
        let at = doc
            .features
            .iter()
            .position(|f| f.id == marker)
            .ok_or(DocError::UnknownFeature(marker))?;
        doc.move_feature(id, at.saturating_add(1))?;
        doc.set_rollback(Some(id))?;
    }
    Ok(id)
}

pub(crate) fn add_feature(
    session: &mut Session,
    command: &'static str,
    name: Option<&str>,
    kind: FeatureKind,
) -> Run {
    let label = format!("Add {}", kind.type_name());
    let id = edit(session, command, &label, |doc| {
        insert_feature(doc, name, kind)
    })?;
    feature_result(session, id)
}

/// The `cut`, `join` and `intersect` parameters of a sweep: the body the
/// swept solid is combined with at once, and how.
pub(crate) const COMBINE: [(&str, BooleanOp); 3] = [
    ("cut", BooleanOp::Subtract),
    ("join", BooleanOp::Union),
    ("intersect", BooleanOp::Intersect),
];

pub(crate) fn combine_param(
    doc: &Document,
    p: &Params,
    command: &str,
) -> Result<Option<(BooleanOp, FeatureId)>, CommandError> {
    let mut found = None;
    for (key, op) in COMBINE {
        if !p.has(key) {
            continue;
        }
        if found.is_some() {
            return fail(format!(
                "{command}: give one of `cut`, `join` and `intersect`, not several"
            ));
        }
        found = Some((op, feature_ref(doc, p, command, key)?));
    }
    Ok(found)
}

/// Adds a sweep (an extrusion, a revolution) and, when it is to be
/// combined with a body at once, the boolean that does it, as one undo
/// step. The name goes to what the caller builds on: the boolean when
/// there is one, else the sweep.
pub(crate) fn add_sweep(
    session: &mut Session,
    command: &'static str,
    name: Option<&str>,
    kind: FeatureKind,
    combine: Option<(BooleanOp, FeatureId)>,
) -> Run {
    let Some((op, target)) = combine else {
        return add_feature(session, command, name, kind);
    };
    let label = format!("Add {} and {}", kind.type_name(), op.name());
    let (sweep, combined) = edit(session, command, &label, |doc| {
        let sweep = insert_feature(doc, None, kind)?;
        let combined = insert_feature(
            doc,
            name,
            FeatureKind::Boolean(Boolean {
                op,
                target,
                tool: sweep,
                keep_tool: false,
            }),
        )?;
        Ok((sweep, combined))
    })?;
    let mut result = feature_result(session, combined)?;
    let swept = feature_result(session, sweep)?;
    if let Some(map) = result.as_object_mut() {
        map.insert("sweep".into(), swept);
    }
    Ok(result)
}

/// What a feature command returns: the feature and how it regenerated, so
/// the caller sees at once whether the change worked.
pub(crate) fn feature_result(session: &mut Session, id: FeatureId) -> Run {
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
        if let Some(root) = craft3d_regen::root_cause(doc, &regen.features, id) {
            map.insert("causedBy".into(), root.to_string().into());
        }
        if let Some(report) = &outcome.sweep {
            map.insert("path".into(), crate::sweep_path_cmds::report_json(report));
        }
        if let Some(report) = &outcome.thread {
            map.insert("thread".into(), crate::helix_cmds::thread_json(report));
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

pub(crate) fn vec3(v: Vec3) -> Value {
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
    let mut open = OpenDocument::new(document, Some(path.to_string()));
    open.disk = crate::recovery::FileStamp::of(&resolved);
    let index = session.add_document(open)?;
    Ok(json!({ "document": index, "path": path, "features": features }))
}

#[cfg(not(target_arch = "wasm32"))]
fn file_save(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("file.save", params, &["path", "overwrite"])?;
    let overwrite = p.bool("overwrite")?.unwrap_or(false);
    let path = match (p.str("path")?, &session.document()?.path) {
        (Some(path), _) => path.to_string(),
        (None, Some(own)) => own.clone(),
        (None, None) => return fail("file.save: this document has no file yet; give a `path`"),
    };
    let resolved = session.workspace.resolve_write(&path)?;
    let open = session.document_mut()?;
    // A recovered document is older than its original file when that
    // file was written after the recovery was taken (by another 3DCraft,
    // or by a save the crashed one made after its last autosave).
    if let Some(recovered) = open.recovered
        && !overwrite
        && open.path.as_deref() == Some(path.as_str())
        && let Some(now) = crate::recovery::FileStamp::of(&resolved)
        && recovered.original != Some(now)
    {
        return fail(format!(
            "file.save: `{path}` changed on disk since this recovered document was autosaved, so saving would replace a file that may be newer; save under another `path`, or pass {{\"overwrite\": true}} to replace it (at the window: File, Save As, and choose the file again)"
        ));
    }
    craft3d_format::save(&resolved, open.history.current())
        .map_err(|e| CommandError::new(format!("file.save: cannot save `{path}`: {e}")))?;
    open.saved_revision = Some(open.history.revision());
    open.path = Some(path.clone());
    open.disk = crate::recovery::FileStamp::of(&resolved);
    open.recovered = None;
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
    if open.close_asks() && !discard {
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
        .filter_map(|(_, body)| mass_of(body).ok())
        .map(|m| m.area)
        .sum();
    if area.is_finite() && area > 0.0 {
        area.sqrt() * 1e-3
    } else {
        1e-3
    }
}

pub(crate) fn tolerance_param(
    p: &Params,
    command: &str,
    regen: &Regen,
) -> Result<TessParams, CommandError> {
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
    let p = Params::new(
        "file.export",
        params,
        &[
            "path",
            "format",
            "tolerance",
            "sketch",
            "face",
            "feature",
            "construction",
        ],
    )?;
    let path = p.required_str("path")?;
    if let Some(format) = crate::drawing_cmds::wanted(p.str("format")?, path) {
        let resolved = session.workspace.resolve_write(path)?;
        return crate::drawing_cmds::export(session, &p, path, &resolved, format);
    }
    // The other formats hold every body.
    let outline_only = || match ["sketch", "face", "feature", "construction"]
        .into_iter()
        .find(|key| p.has(key))
    {
        Some(key) => fail(format!(
            "file.export: `{key}` is for the outline of a sketch or a face, which is a .dxf or .svg file; the other formats hold every body"
        )),
        None => Ok(()),
    };
    if crate::step_cmds::wanted(p.str("format")?, path) {
        outline_only()?;
        if p.number("tolerance")?.is_some() {
            return fail(
                "file.export: `tolerance` is for mesh formats; STEP is exact and carries the document's tolerance",
            );
        }
        let resolved = session.workspace.resolve_write(path)?;
        return crate::step_cmds::export(session, path, &resolved);
    }
    let format = match p.str("format")? {
        Some("stl") => MeshFormat::StlBinary,
        Some("stl-ascii") => MeshFormat::StlAscii,
        Some("3mf") => MeshFormat::ThreeMf,
        Some("obj") => MeshFormat::Obj,
        Some(other) => return fail(format!("file.export: unknown format `{other}`; use step, stl, stl-ascii, 3mf, obj, dxf or svg")),
        None => std::path::Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(MeshFormat::from_extension)
            .ok_or_else(|| {
                CommandError::new(format!(
                    "file.export: cannot tell the format from `{path}`; use a .step, .stp, .stl, .3mf, .obj, .dxf or .svg name, or pass `format`"
                ))
            })?,
    };
    outline_only()?;
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
    let mut closed = 0;
    for (name, mesh) in &meshes {
        let object = craft3d_io::to_object(mesh, name)
            .map_err(|e| CommandError::new(format!("file.export: {name}: {e}")))?;
        triangles += object.triangles.len();
        if object.kind == craft3d_meshio::ObjectKind::Solid {
            closed += 1;
        }
        model.objects.push(object);
    }
    let bytes = craft3d_io::encode(&model, format)
        .map_err(|e| CommandError::new(format!("file.export: {e}")))?;
    crate::workspace::write_atomic(&resolved, &bytes)?;
    Ok(json!({
        "path": path,
        "bytes": bytes.len(),
        "bodies": model.objects.len(),
        // How many of the meshes are closed and consistently oriented.
        "closed": closed,
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

/// The `axis` and `angle` parameters shared by the primitives that can be
/// turned; absent means upright.
fn orientation(p: &Params) -> Result<Orientation, CommandError> {
    let default = Orientation::default();
    Ok(Orientation {
        axis: p.expr3("axis")?.map(exprs).unwrap_or(default.axis),
        angle: p.expr("angle")?.map(Expr::new).unwrap_or(default.angle),
    })
}

pub(crate) fn required(p: &Params, key: &str) -> Result<Expr, CommandError> {
    Ok(Expr::new(p.expr(key)?.ok_or_else(|| p.missing(key))?))
}

fn position(p: &Params) -> Result<[Expr; 3], CommandError> {
    Ok(exprs(p.expr3("position")?.unwrap_or_else(origin)))
}

fn feature_block(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.block",
        params,
        &["size", "position", "axis", "angle", "name"],
    )?;
    let size = p.expr3("size")?.ok_or_else(|| p.missing("size"))?;
    let kind = FeatureKind::Block(Block {
        position: position(&p)?,
        size: exprs(size),
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.block", p.str("name")?, kind)
}

fn feature_cylinder(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.cylinder",
        params,
        &["radius", "height", "position", "axis", "angle", "name"],
    )?;
    let kind = FeatureKind::Cylinder(Cylinder {
        position: position(&p)?,
        radius: required(&p, "radius")?,
        height: required(&p, "height")?,
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.cylinder", p.str("name")?, kind)
}

fn feature_sphere(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("feature.sphere", params, &["radius", "position", "name"])?;
    let kind = FeatureKind::Sphere(Sphere {
        position: position(&p)?,
        radius: required(&p, "radius")?,
    });
    add_feature(session, "feature.sphere", p.str("name")?, kind)
}

fn feature_cone(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.cone",
        params,
        &["radius", "height", "position", "axis", "angle", "name"],
    )?;
    let kind = FeatureKind::Cone(Cone {
        position: position(&p)?,
        radius: required(&p, "radius")?,
        height: required(&p, "height")?,
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.cone", p.str("name")?, kind)
}

fn feature_torus(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.torus",
        params,
        &["major", "minor", "position", "axis", "angle", "name"],
    )?;
    let kind = FeatureKind::Torus(Torus {
        position: position(&p)?,
        major: required(&p, "major")?,
        minor: required(&p, "minor")?,
        orientation: orientation(&p)?,
    });
    add_feature(session, "feature.torus", p.str("name")?, kind)
}

fn feature_extrude(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "feature.extrude",
        params,
        &[
            "sketch",
            "distance",
            "throughAll",
            "upTo",
            "symmetric",
            "reversed",
            "cut",
            "join",
            "intersect",
            "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, "feature.extrude", "sketch")?;
    doc.sketch(sketch).map_err(doc_error("feature.extrude"))?;
    let combine = combine_param(doc, &p, "feature.extrude")?;
    let Some(end) = extrude_end(&p, "feature.extrude")? else {
        return fail(
            "feature.extrude: missing parameter `distance` (or `throughAll` or `upTo`: where the extrusion ends)",
        );
    };
    let symmetric = p.bool("symmetric")?.unwrap_or(false);
    symmetric_fits(&end, symmetric, "feature.extrude")?;
    let kind = FeatureKind::Extrude(Extrude {
        sketch,
        // An end that is not a distance keeps one, unused.
        distance: Expr::new(p.expr("distance")?.unwrap_or_else(|| "0".to_string())),
        symmetric,
        end,
        reversed: p.bool("reversed")?.unwrap_or(false),
    });
    add_sweep(session, "feature.extrude", p.str("name")?, kind, combine)
}

/// The end of an extrusion as the parameters give it: one of `distance`,
/// `throughAll` (true) and `upTo`. `None` when none of them is given.
fn extrude_end(p: &Params, command: &str) -> Result<Option<ExtrudeEnd>, CommandError> {
    let distance = p.expr("distance")?.is_some();
    let through = p.bool("throughAll")?;
    let up_to = match p.str("upTo")? {
        None => None,
        Some(text) => Some(
            text.parse::<PersistentName>()
                .map_err(|reason| CommandError::new(format!("{command}: `upTo`: {reason}")))?,
        ),
    };
    if up_to.as_ref().is_some_and(|name| !name.is_face()) {
        return fail(format!(
            "{command}: `upTo` must name a face (see faceNames in document.inspect)"
        ));
    }
    let given =
        usize::from(distance) + usize::from(through == Some(true)) + usize::from(up_to.is_some());
    if given > 1 {
        return fail(format!(
            "{command}: give one of `distance`, `throughAll` and `upTo`, not several"
        ));
    }
    Ok(match (up_to, through) {
        (Some(face), _) => Some(ExtrudeEnd::UpToFace { face }),
        (None, Some(true)) => Some(ExtrudeEnd::ThroughAll),
        // `throughAll: false` on its own asks for a distance again.
        (None, Some(false)) => Some(ExtrudeEnd::Distance),
        (None, None) => distance.then_some(ExtrudeEnd::Distance),
    })
}

/// An extrusion up to a face has one end: it cannot be symmetric.
fn symmetric_fits(end: &ExtrudeEnd, symmetric: bool, command: &str) -> Result<(), CommandError> {
    if symmetric && matches!(end, ExtrudeEnd::UpToFace { .. }) {
        return fail(format!(
            "{command}: `symmetric` does not go with `upTo`: an extrusion up to a face goes one way"
        ));
    }
    Ok(())
}

/// The parameters `feature.edit` can change, by name.
/// The parameters of a thread that `feature.edit` takes as null.
const THREAD_NULLS: [&str; 2] = ["size", "pitch"];

const EDITABLE: [&str; 43] = [
    "sketch",
    "sections",
    "ruled",
    "closed",
    "source",
    "target",
    "tool",
    "op",
    "size",
    "position",
    "axis",
    "radius",
    "height",
    "major",
    "minor",
    "angle",
    "distance",
    "throughAll",
    "upTo",
    "symmetric",
    "reversed",
    "translate",
    "pivot",
    "point",
    "normal",
    "plane",
    "step",
    "count",
    "keep",
    "keepTool",
    "edges",
    "second",
    "chain",
    "path",
    "faces",
    "thickness",
    "open",
    "pitch",
    "turns",
    "leftHanded",
    "length",
    "cosmetic",
    "modeled",
];

fn feature_edit(session: &mut Session, params: &Value) -> Run {
    let mut allowed = vec!["feature"];
    allowed.extend(EDITABLE);
    let p = Params::new("feature.edit", params, &allowed)?;
    let id = feature_ref(
        session.document()?.history.current(),
        &p,
        "feature.edit",
        "feature",
    )?;
    // A thread's size and pitch may be given as null: back to the size
    // of the face, the coarse pitch of the size.
    let is_thread = matches!(
        session
            .document()?
            .history
            .current()
            .feature(id)
            .map(|f| &f.kind),
        Some(FeatureKind::Thread(_))
    );
    // Validate the shape of everything given before touching anything.
    // An axis given as a string is a datum axis, for a circular pattern.
    // (For every other kind an axis is a direction, and a string is
    // refused as one.)
    let circular = matches!(
        session
            .document()?
            .history
            .current()
            .feature(id)
            .map(|f| &f.kind),
        Some(FeatureKind::Pattern(craft3d_doc::Pattern {
            layout: PatternLayout::Circular { .. },
            ..
        }))
    );
    let new_axis = match p.get("axis") {
        Some(Value::String(_)) if circular => Some(crate::datum_cmds::datum_ref(
            session.document()?.history.current(),
            &p,
            "feature.edit",
            "axis",
            "datumAxis",
        )?),
        _ => None,
    };
    // The axis of a revolution is one of the sketch's own, a line of the
    // sketch or a datum axis, as feature.revolve takes it.
    let revolution = matches!(
        session
            .document()?
            .history
            .current()
            .feature(id)
            .map(|f| &f.kind),
        Some(FeatureKind::Revolve(_))
    );
    let new_revolve_axis = match p.get("axis") {
        Some(value) if revolution => Some(crate::sweep_cmds::revolve_axis(
            session.document()?.history.current(),
            "feature.edit",
            value,
        )?),
        _ => None,
    };
    // The sections of a loft, as feature.loft takes them.
    let new_sections = match p.get("sections") {
        Some(value) => Some(crate::loft_cmds::loft_sections(
            session.document()?.history.current(),
            "feature.edit",
            value,
        )?),
        None => None,
    };
    // What the feature is made from: another sketch, another body. Each
    // has to be there now; whether it can be used (it comes earlier, it
    // has a body nothing else took) is for regeneration to say, as when
    // the feature was added.
    let mut made_from: [(&str, Option<FeatureId>); 5] = [
        ("sketch", None),
        ("source", None),
        ("target", None),
        ("tool", None),
        ("path", None),
    ];
    for (key, found) in &mut made_from {
        if p.get(key).is_some() {
            let doc = session.document()?.history.current();
            let other = feature_ref(doc, &p, "feature.edit", key)?;
            if other == id {
                return fail(format!(
                    "feature.edit: `{key}` is the feature itself; a feature cannot be made from itself"
                ));
            }
            if *key == "sketch" {
                doc.sketch(other).map_err(doc_error("feature.edit"))?;
            }
            if *key == "path" {
                crate::sweep_path_cmds::path_ref(doc, other, "feature.edit")?;
            }
            *found = Some(other);
        }
    }
    let [
        (_, new_sketch),
        (_, new_source),
        (_, new_target),
        (_, new_tool),
        (_, new_path),
    ] = made_from;
    let new_op = match p.str("op")? {
        None => None,
        Some(op) => Some(BooleanOp::from_name(op).ok_or_else(|| {
            CommandError::new(format!(
                "feature.edit: unknown op `{op}`; use union, subtract or intersect"
            ))
        })?),
    };
    for key in [
        "size",
        "position",
        "axis",
        "translate",
        "pivot",
        "point",
        "normal",
        "step",
    ] {
        if key == "axis" && (new_axis.is_some() || new_revolve_axis.is_some()) {
            continue;
        }
        // The size of a thread is a designation (`M10`), checked with
        // the kind.
        if key == "size" && (is_thread || matches!(p.get("size"), Some(Value::String(_)))) {
            continue;
        }
        p.expr3(key)?;
    }
    for key in [
        "radius",
        "height",
        "major",
        "minor",
        "angle",
        "distance",
        "count",
        "second",
        "thickness",
    ] {
        p.expr(key)?;
    }
    for key in [
        "symmetric",
        "reversed",
        "keep",
        "keepTool",
        "chain",
        "modeled",
        "cosmetic",
    ] {
        p.bool(key)?;
    }
    let new_end = extrude_end(&p, "feature.edit")?;
    let new_plane = match p.get("plane") {
        None => None,
        Some(_) => Some(crate::datum_cmds::datum_ref(
            session.document()?.history.current(),
            &p,
            "feature.edit",
            "plane",
            "datumPlane",
        )?),
    };
    let given: Vec<&str> = EDITABLE
        .into_iter()
        .filter(|key| {
            p.get(key).is_some() || (is_thread && THREAD_NULLS.contains(key) && p.has(key))
        })
        .collect();
    if given.is_empty() {
        return fail(format!(
            "feature.edit: give at least one parameter to change ({})",
            EDITABLE.join(", ")
        ));
    }
    let mut kind = session
        .document()?
        .history
        .current()
        .feature(id)
        .map(|f| f.kind.clone())
        .ok_or_else(|| CommandError::new(format!("feature.edit: there is no feature {id}")))?;
    // Each kind lists the parameters it has; a given one outside the list
    // is an error rather than ignored.
    let (name, own): (&str, Vec<(&str, Slot)>) = match &mut kind {
        FeatureKind::Sketch(_) => {
            return fail(
                "feature.edit: a sketch is changed with the sketch.* commands (sketch.edit, sketch.constrain, ...)",
            );
        }
        FeatureKind::Block(b) => (
            "block",
            vec![
                ("position", Slot::Triple(&mut b.position)),
                ("size", Slot::Triple(&mut b.size)),
                ("axis", Slot::Triple(&mut b.orientation.axis)),
                ("angle", Slot::One(&mut b.orientation.angle)),
            ],
        ),
        FeatureKind::Cylinder(c) => (
            "cylinder",
            vec![
                ("position", Slot::Triple(&mut c.position)),
                ("radius", Slot::One(&mut c.radius)),
                ("height", Slot::One(&mut c.height)),
                ("axis", Slot::Triple(&mut c.orientation.axis)),
                ("angle", Slot::One(&mut c.orientation.angle)),
            ],
        ),
        FeatureKind::Sphere(s) => (
            "sphere",
            vec![
                ("position", Slot::Triple(&mut s.position)),
                ("radius", Slot::One(&mut s.radius)),
            ],
        ),
        FeatureKind::Cone(c) => (
            "cone",
            vec![
                ("position", Slot::Triple(&mut c.position)),
                ("radius", Slot::One(&mut c.radius)),
                ("height", Slot::One(&mut c.height)),
                ("axis", Slot::Triple(&mut c.orientation.axis)),
                ("angle", Slot::One(&mut c.orientation.angle)),
            ],
        ),
        FeatureKind::Torus(t) => (
            "torus",
            vec![
                ("position", Slot::Triple(&mut t.position)),
                ("major", Slot::One(&mut t.major)),
                ("minor", Slot::One(&mut t.minor)),
                ("axis", Slot::Triple(&mut t.orientation.axis)),
                ("angle", Slot::One(&mut t.orientation.angle)),
            ],
        ),
        FeatureKind::Extrude(e) => (
            "extrude",
            vec![
                ("sketch", Slot::Elsewhere),
                ("distance", Slot::One(&mut e.distance)),
                ("symmetric", Slot::Flag(&mut e.symmetric)),
                ("reversed", Slot::Flag(&mut e.reversed)),
                // Applied below: the three name one thing, the end.
                ("throughAll", Slot::Elsewhere),
                ("upTo", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Revolve(r) => (
            "revolve",
            vec![
                ("sketch", Slot::Elsewhere),
                ("axis", Slot::Elsewhere),
                ("angle", Slot::One(&mut r.angle)),
                ("symmetric", Slot::Flag(&mut r.symmetric)),
            ],
        ),
        FeatureKind::Loft(l) => (
            "loft",
            vec![
                ("sections", Slot::Elsewhere),
                ("ruled", Slot::Flag(&mut l.ruled)),
                ("closed", Slot::Flag(&mut l.closed)),
            ],
        ),
        FeatureKind::Sweep(_) => (
            "sweep",
            vec![("sketch", Slot::Elsewhere), ("path", Slot::Elsewhere)],
        ),
        FeatureKind::Transform(t) => (
            "transform",
            vec![
                ("source", Slot::Elsewhere),
                ("translate", Slot::Triple(&mut t.translate)),
                ("pivot", Slot::Triple(&mut t.pivot)),
                ("axis", Slot::Triple(&mut t.axis)),
                ("angle", Slot::One(&mut t.angle)),
                ("keep", Slot::Flag(&mut t.keep)),
            ],
        ),
        FeatureKind::Boolean(b) => (
            "boolean",
            vec![
                ("op", Slot::Elsewhere),
                ("target", Slot::Elsewhere),
                ("tool", Slot::Elsewhere),
                ("keepTool", Slot::Flag(&mut b.keep_tool)),
            ],
        ),
        FeatureKind::Chamfer(c) => (
            "chamfer",
            vec![
                // Applied below: together they are the size's form.
                ("edges", Slot::Elsewhere),
                ("distance", Slot::Elsewhere),
                ("second", Slot::Elsewhere),
                ("angle", Slot::Elsewhere),
                ("chain", Slot::Flag(&mut c.chain)),
            ],
        ),
        FeatureKind::Fillet(f) => (
            "fillet",
            vec![
                // Applied below, with the edges of a chamfer.
                ("edges", Slot::Elsewhere),
                ("radius", Slot::One(&mut f.radius)),
                ("chain", Slot::Flag(&mut f.chain)),
            ],
        ),
        FeatureKind::OffsetFace(o) => (
            "offset face",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
                ("distance", Slot::One(&mut o.distance)),
            ],
        ),
        FeatureKind::DeleteFace(_) => (
            "delete face",
            vec![
                // Applied below.
                ("faces", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Shell(s) => (
            "shell",
            vec![
                ("source", Slot::Elsewhere),
                ("thickness", Slot::One(&mut s.thickness)),
                // Applied below.
                ("open", Slot::Elsewhere),
            ],
        ),
        FeatureKind::Mirror(m) => (
            "mirror",
            vec![
                ("source", Slot::Elsewhere),
                ("point", Slot::Triple(&mut m.point)),
                ("normal", Slot::Triple(&mut m.normal)),
                // Applied below: a datum plane instead of point and normal.
                ("plane", Slot::Elsewhere),
                ("keep", Slot::Flag(&mut m.keep)),
            ],
        ),
        FeatureKind::DatumPoint(datum) => (
            "datum point",
            match &mut datum.at {
                craft3d_doc::PointRef::At { position } => {
                    vec![("position", Slot::Triple(position))]
                }
                _ => {
                    return fail(
                        "feature.edit: a datum point at a vertex or a centre has no parameter to change here; give it another definition with datum.point and replace",
                    );
                }
            },
        ),
        FeatureKind::Thread(thread) => (
            "thread",
            vec![
                // Applied below: a designation or an expression, and
                // either may be null (back to the face's size, the
                // coarse pitch).
                ("size", Slot::Elsewhere),
                ("pitch", Slot::Elsewhere),
                ("length", Slot::One(&mut thread.length)),
                ("leftHanded", Slot::Flag(&mut thread.left_handed)),
                // Applied below: either says whether it is cut.
                ("modeled", Slot::Elsewhere),
                ("cosmetic", Slot::Elsewhere),
            ],
        ),
        FeatureKind::DatumHelix(helix) => {
            let mut slots = match &mut helix.start {
                craft3d_doc::HelixStart::Radius { radius } => vec![("radius", Slot::One(radius))],
                craft3d_doc::HelixStart::Point { .. } => Vec::new(),
            };
            slots.extend([
                ("pitch", Slot::One(&mut helix.pitch)),
                // Applied below: either one is how far the helix goes.
                ("turns", Slot::Elsewhere),
                ("height", Slot::Elsewhere),
                ("leftHanded", Slot::Flag(&mut helix.left_handed)),
            ]);
            ("datum helix", slots)
        }
        FeatureKind::DatumAxis(_) => {
            return fail(
                "feature.edit: a datum axis has no parameter to change here; give it another definition with datum.axis and replace",
            );
        }
        FeatureKind::DatumPlane(datum) => (
            "datum plane",
            match &mut datum.by {
                PlaneBy::Offset { distance, .. } => vec![("distance", Slot::One(distance))],
                PlaneBy::Angle { angle, .. } => vec![("angle", Slot::One(angle))],
                PlaneBy::ThreePoints { .. } => {
                    return fail(
                        "feature.edit: a datum plane through three points has no parameter to change here; give it other points with datum.plane and replace",
                    );
                }
            },
        ),
        FeatureKind::Pattern(pattern) => {
            let mut slots = vec![
                ("source", Slot::Elsewhere),
                ("count", Slot::One(&mut pattern.count)),
            ];
            match &mut pattern.layout {
                PatternLayout::Linear { step } => {
                    slots.push(("step", Slot::Triple(step)));
                    ("linear pattern", slots)
                }
                PatternLayout::Circular {
                    point, axis, angle, ..
                } => {
                    slots.push(("point", Slot::Triple(point)));
                    slots.push(("axis", Slot::Triple(axis)));
                    slots.push(("angle", Slot::One(angle)));
                    ("circular pattern", slots)
                }
            }
        }
    };
    let names: Vec<&str> = own.iter().map(|(key, _)| *key).collect();
    if let Some(unused) = given.iter().find(|key| !names.contains(key)) {
        return fail(format!(
            "feature.edit: `{unused}` is not a parameter of a {name}; its parameters are: {}",
            names.join(", ")
        ));
    }
    for (key, slot) in own {
        match slot {
            // Applied below: a datum axis instead of point and axis.
            Slot::Triple(_) if key == "axis" && new_axis.is_some() => {}
            Slot::Triple(target) => {
                if let Some(value) = p.expr3(key)? {
                    *target = exprs(value);
                }
            }
            Slot::One(target) => {
                if let Some(value) = p.expr(key)? {
                    *target = Expr::new(value);
                }
            }
            Slot::Flag(target) => {
                if let Some(value) = p.bool(key)? {
                    *target = value;
                }
            }
            Slot::Elsewhere => {}
        }
    }
    match (&mut kind, new_axis) {
        (
            FeatureKind::Pattern(craft3d_doc::Pattern {
                layout: PatternLayout::Circular { about, .. },
                ..
            }),
            new_axis,
        ) => match new_axis {
            Some(_) if p.get("point").is_some() => {
                return fail(
                    "feature.edit: give either `axis` as a datum axis or `axis` as a direction and `point`, not both",
                );
            }
            Some(axis) => *about = Some(axis),
            // A line of its own again when a direction or point is given.
            None if p.get("axis").is_some() || p.get("point").is_some() => *about = None,
            None => {}
        },
        (_, Some(_)) => {
            return fail(
                "feature.edit: `axis` as a datum axis goes with a circular pattern; this feature's `axis` is a direction [x, y, z]",
            );
        }
        (_, None) => {}
    }
    if let FeatureKind::Thread(thread) = &mut kind {
        if p.has("size") {
            thread.size = match p.get("size") {
                Some(size) => Some(crate::helix_cmds::size_value("feature.edit", size)?),
                None => None,
            };
        }
        if p.has("pitch") {
            thread.pitch = p.expr("pitch")?.map(Expr::new);
        }
        if let Some(modeled) = crate::helix_cmds::modeled_value("feature.edit", &p)? {
            thread.modeled = modeled;
        }
    }
    if let FeatureKind::DatumHelix(helix) = &mut kind {
        match (p.expr("turns")?, p.expr("height")?) {
            (Some(_), Some(_)) => {
                return fail(
                    "feature.edit: give either `turns` or `height`: both say how far the helix goes",
                );
            }
            (Some(turns), None) => {
                helix.extent = craft3d_doc::HelixExtent::Turns {
                    turns: Expr::new(turns),
                };
            }
            (None, Some(height)) => {
                helix.extent = craft3d_doc::HelixExtent::Height {
                    height: Expr::new(height),
                };
            }
            (None, None) => {}
        }
    }
    if let FeatureKind::Mirror(mirror) = &mut kind {
        // A plane of its own again when point or normal is given.
        match new_plane {
            Some(_) if p.get("normal").is_some() || p.get("point").is_some() => {
                return fail(
                    "feature.edit: give either `plane` (a datum plane) or `normal` and `point`, not both",
                );
            }
            Some(plane) => mirror.plane = Some(plane),
            None if p.get("normal").is_some() || p.get("point").is_some() => mirror.plane = None,
            None => {}
        }
    }
    // What the feature is made from. (A parameter the kind does not have
    // was refused above.)
    match &mut kind {
        FeatureKind::Extrude(extrusion) => {
            extrusion.sketch = new_sketch.unwrap_or(extrusion.sketch);
        }
        FeatureKind::Revolve(revolution) => {
            revolution.sketch = new_sketch.unwrap_or(revolution.sketch);
            revolution.axis = new_revolve_axis.unwrap_or(revolution.axis);
        }
        FeatureKind::Loft(loft) => {
            if let Some(sections) = new_sections {
                loft.sections = sections;
            }
            if loft.closed && loft.sections.len() < 3 {
                return fail("feature.edit: a closed loft passes through at least 3 sketches");
            }
        }
        FeatureKind::Sweep(sweep) => {
            sweep.sketch = new_sketch.unwrap_or(sweep.sketch);
            sweep.path = new_path.unwrap_or(sweep.path);
            if sweep.sketch == sweep.path {
                return fail(
                    "feature.edit: `sketch` and `path` are the same sketch; the path is drawn in a sketch of its own",
                );
            }
        }
        FeatureKind::Transform(transform) => {
            transform.source = new_source.unwrap_or(transform.source);
        }
        FeatureKind::Mirror(mirror) => mirror.source = new_source.unwrap_or(mirror.source),
        FeatureKind::Pattern(pattern) => pattern.source = new_source.unwrap_or(pattern.source),
        FeatureKind::Boolean(boolean) => {
            boolean.op = new_op.unwrap_or(boolean.op);
            boolean.target = new_target.unwrap_or(boolean.target);
            boolean.tool = new_tool.unwrap_or(boolean.tool);
            if boolean.target == boolean.tool {
                return fail(
                    "feature.edit: `target` and `tool` are the same feature; a body cannot be combined with itself",
                );
            }
        }
        FeatureKind::Chamfer(chamfer) => crate::blend_cmds::edit(&p, chamfer)?,
        FeatureKind::Fillet(fillet) => crate::blend_cmds::edit_fillet(&p, fillet)?,
        FeatureKind::OffsetFace(offset) => crate::face_cmds::edit_offset(&p, offset)?,
        FeatureKind::DeleteFace(deletion) => crate::face_cmds::edit_delete(&p, deletion)?,
        FeatureKind::Shell(shell) => {
            shell.source = new_source.unwrap_or(shell.source);
            crate::face_cmds::edit_shell(&p, shell)?;
        }
        _ => {}
    }
    crate::blend_cmds::of_one_body("feature.edit", session, &kind)?;
    if let (FeatureKind::Extrude(extrusion), Some(end)) = (&mut kind, new_end) {
        extrusion.end = end;
    }
    if let FeatureKind::Extrude(extrusion) = &kind {
        symmetric_fits(&extrusion.end, extrusion.symmetric, "feature.edit")?;
    }
    edit(session, "feature.edit", "Edit feature", |doc| {
        doc.feature_mut(id)?.kind = kind;
        Ok(())
    })?;
    feature_result(session, id)
}

/// Where a parameter of a feature lives.
enum Slot<'a> {
    One(&'a mut Expr),
    Triple(&'a mut [Expr; 3]),
    Flag(&'a mut bool),
    /// A parameter that is not one field: handled where the kind is known.
    Elsewhere,
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
    let bbox_tess = TessParams {
        chordal_tolerance: automatic_tolerance(&regen),
        ..TessParams::default()
    };
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
        // Failed because something it is made from failed: the feature at
        // the start of that chain, the one to fix.
        if let (Some(map), Some(root)) = (
            entry.as_object_mut(),
            craft3d_regen::root_cause(doc, &regen.features, feature.id),
        ) {
            map.insert("causedBy".into(), root.to_string().into());
        }
        // A sketch: how far it is constrained. sketch.inspect has the rest.
        if let (Some(map), Some(solved)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.sketch.as_ref()),
        ) {
            map.insert("dof".into(), solved.dof.map_or(Value::Null, Value::from));
        }
        // Reference geometry: where it lies.
        if let (Some(map), Some(datum)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.datum.as_ref()),
        ) {
            map.insert("datum".into(), crate::datum_cmds::datum_json(datum));
        }
        // A sweep: how its path was followed.
        if let (Some(map), Some(report)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.sweep.as_ref()),
        ) {
            map.insert("path".into(), crate::sweep_path_cmds::report_json(report));
        }
        // A thread: what it is.
        if let (Some(map), Some(report)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.thread.as_ref()),
        ) {
            map.insert("thread".into(), crate::helix_cmds::thread_json(report));
        }
        // Faces or edges given by the names of an earlier body, followed
        // to the newest one: the parameters as the feature was built.
        if let (Some(map), Some(built)) = (
            entry.as_object_mut(),
            outcome.and_then(|o| o.followed.as_ref()),
        ) {
            map.insert(
                "followed".into(),
                serde_json::to_value(&built.kind).unwrap_or(Value::Null),
            );
        }
        // A body that went into a later feature is no longer listed under
        // bodies; this says where it went.
        if let (Some(map), Some(taker)) =
            (entry.as_object_mut(), outcome.and_then(|o| o.consumed_by))
        {
            map.insert("bodyUsedBy".into(), taker.to_string().into());
        }
        features.push(entry);
        if let Some(body) = outcome.and_then(|o| o.own_body()) {
            let mut entry = json!({
                "feature": feature.id.to_string(),
                "name": feature.name,
                "kind": match body.kind { BodyKind::Solid => "solid", BodyKind::Sheet => "sheet" },
                "valid": true,
                "faces": body.faces.len(),
                "edges": body.edges.len(),
                "vertices": body.vertices.len(),
            });
            if let (Some(map), Ok(mass)) = (entry.as_object_mut(), mass_of(body)) {
                map.insert("volume".into(), mass.volume.into());
                map.insert("area".into(), mass.area.into());
                map.insert("centroid".into(), mass.centroid.map_or(Value::Null, vec3));
            }
            // What each face is called by the features that come after:
            // a name made of where the face came from, not of its index.
            // A face a boolean merged from faces of both operands answers
            // to the names of the others too: listed per face as aliases,
            // only when there are any.
            if let (Some(map), Some(names)) = (
                entry.as_object_mut(),
                outcome.and_then(|o| o.names.as_ref()),
            ) {
                let listed: Vec<String> = names.faces.iter().map(ToString::to_string).collect();
                map.insert("faceNames".into(), listed.into());
                if !names.aliases.is_empty() {
                    let aliases: Vec<Vec<String>> = (0..names.faces.len())
                        .map(|face| {
                            names
                                .aliases_of(face)
                                .iter()
                                .map(ToString::to_string)
                                .collect()
                        })
                        .collect();
                    map.insert("faceAliases".into(), json!(aliases));
                }
            }
            // From a tessellation: exact for flat-faced bodies, inside the
            // true box by at most the tolerance for curved ones.
            if let (Some(map), Ok(mesh)) =
                (entry.as_object_mut(), tessellate_body(body, &bbox_tess))
                && let Some((lo, hi)) = mesh.bbox().corners()
            {
                map.insert("bbox".into(), json!([vec3(lo), vec3(hi)]));
                map.insert("bboxTolerance".into(), bbox_tess.chordal_tolerance.into());
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
        let mass = mass_of(body)
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

fn document_check(session: &mut Session, params: &Value) -> Run {
    let p = Params::new("document.check", params, &["deep"])?;
    let deep = p.bool("deep")?.unwrap_or(false);
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let tolerance = craft3d_math::Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or_else(|| {
            CommandError::new("document.check: the document's tolerance is not positive and finite")
        })?;
    let mut problems = Vec::new();
    for variable in &regen.variables {
        if let Err(error) = &variable.value {
            problems.push(json!({ "variable": variable.name, "problem": error }));
        }
    }
    let mut checked = 0;
    let mut marginal = 0_usize;
    for outcome in &regen.features {
        let name = doc
            .feature(outcome.id)
            .map_or_else(|| outcome.id.to_string(), |f| f.name.clone());
        if let Status::Failed(reason) = &outcome.status {
            let mut problem =
                json!({ "feature": outcome.id.to_string(), "name": name, "problem": reason });
            if let (Some(map), Some(root)) = (
                problem.as_object_mut(),
                craft3d_regen::root_cause(doc, &regen.features, outcome.id),
            ) {
                map.insert("causedBy".into(), root.to_string().into());
            }
            problems.push(problem);
        }
        if let Some(body) = outcome.own_body() {
            checked += 1;
            for defect in craft3d_topo::check(body, &tolerance) {
                problems.push(json!({
                    "feature": outcome.id.to_string(), "name": name, "problem": format!("invalid body: {defect}"),
                }));
            }
            // An unbounded or otherwise unmeasurable face shows up here.
            if let Err(error) = mass_of(body) {
                problems.push(json!({
                    "feature": outcome.id.to_string(), "name": name, "problem": format!("cannot be measured: {error}"),
                }));
            }
            if deep {
                let params = craft3d_tess::SelfCheckParams::new(doc.tolerance.linear);
                match craft3d_tess::body_self_intersections(body, &params) {
                    Ok(found) => {
                        marginal = marginal
                            .saturating_add(found.marginal_crossings)
                            .saturating_add(found.shells.iter().filter(|s| !s.certain).count());
                        if !found.is_clean() {
                            problems.push(json!({
                                "feature": outcome.id.to_string(), "name": name,
                                "problem": format!("the body passes through itself: {found}"),
                            }));
                        }
                    }
                    Err(error) => problems.push(json!({
                        "feature": outcome.id.to_string(), "name": name,
                        "problem": format!("cannot be searched for self-intersections: {error}"),
                    })),
                }
            }
        }
    }
    let mut result =
        json!({ "ok": problems.is_empty(), "bodiesChecked": checked, "problems": problems });
    if let (true, Some(map)) = (deep, result.as_object_mut()) {
        map.insert("deep".into(), true.into());
        map.insert("marginal".into(), marginal.into());
    }
    Ok(result)
}

// ------------------------------------------------------------------- view

/// The `box` of `view.render`: two opposite corners, in any order.
fn view_box(value: &Value, command: &str) -> Result<craft3d_math::Bbox3, CommandError> {
    const SHAPE: &str = "`box` must be two opposite corners, [[x, y, z], [x, y, z]], \
                         of numbers within ±1e12 that are not the same point";
    let corner = |value: &Value| -> Option<Vec3> {
        let [x, y, z] = value.as_array()?.as_slice() else {
            return None;
        };
        let number = |v: &Value| v.as_f64().filter(|n| n.is_finite() && n.abs() <= 1e12);
        Some(Vec3::new(number(x)?, number(y)?, number(z)?))
    };
    let corners = value
        .as_array()
        .and_then(|corners| match corners.as_slice() {
            [a, b] => Some((corner(a)?, corner(b)?)),
            _ => None,
        });
    match corners {
        Some((a, b)) if a != b => Ok(craft3d_math::Bbox3::from_points([a, b])),
        _ => fail(format!("{command}: {SHAPE}")),
    }
}

/// How `view.render` looks at the model: the parameters `view`,
/// `projection`, `zoom` and `box`, shared with `view.pick`, which answers
/// what is at a position of the picture these parameters give.
pub(crate) struct Framing {
    command: &'static str,
    pub view: &'static str,
    toward_eye: Vec3,
    up: Vec3,
    pub orthographic: bool,
    pub zoom: f64,
    pub look_at: Option<craft3d_math::Bbox3>,
}

impl Framing {
    /// The keys of the parameters it reads.
    pub const KEYS: [&'static str; 4] = ["view", "projection", "zoom", "box"];

    pub fn from_params(p: &Params, command: &'static str) -> Result<Self, CommandError> {
        const VIEWS: [(&str, Vec3, Vec3); 7] = [
            ("iso", Vec3::new(1.0, -1.0, 0.8), Vec3::Z),
            ("front", Vec3::new(0.0, -1.0, 0.0), Vec3::Z),
            ("back", Vec3::new(0.0, 1.0, 0.0), Vec3::Z),
            ("right", Vec3::new(1.0, 0.0, 0.0), Vec3::Z),
            ("left", Vec3::new(-1.0, 0.0, 0.0), Vec3::Z),
            ("top", Vec3::new(0.0, 0.0, 1.0), Vec3::Y),
            ("bottom", Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
        ];
        let name = p.str("view")?.unwrap_or("iso");
        let Some(&(view, toward_eye, up)) = VIEWS.iter().find(|(view, ..)| *view == name) else {
            return fail(format!(
                "{command}: unknown view `{name}`; use iso, front, back, left, right, top or bottom"
            ));
        };
        let orthographic = match p.str("projection")?.unwrap_or("perspective") {
            "perspective" => false,
            "orthographic" => true,
            other => {
                return fail(format!(
                    "{command}: unknown projection `{other}`; use perspective or orthographic"
                ));
            }
        };
        let zoom = match p.number("zoom")? {
            None => 1.0,
            Some(zoom) if (0.01..=1000.0).contains(&zoom) => zoom,
            Some(_) => {
                return fail(format!(
                    "{command}: `zoom` must be a number from 0.01 to 1000"
                ));
            }
        };
        let look_at = p
            .get("box")
            .map(|value| view_box(value, command))
            .transpose()?;
        Ok(Self {
            command,
            view,
            toward_eye,
            up,
            orthographic,
            zoom,
            look_at,
        })
    }

    /// The direction from the model towards the eye, and up.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn direction(&self) -> (Vec3, Vec3) {
        (self.toward_eye, self.up)
    }

    /// The camera for a picture of `aspect` (width over height) of a model
    /// that fills `model`.
    pub fn camera(&self, model: &craft3d_math::Bbox3, aspect: f64) -> Result<Camera, CommandError> {
        let framed = self.look_at.unwrap_or(*model);
        if self.orthographic {
            Camera::framing_orthographic(&framed, self.toward_eye, self.up, aspect)
        } else {
            // A close-up is taken from outside the whole model.
            Camera::framing_clear_of(&framed, model, self.toward_eye, self.up, 30.0, aspect)
        }
        .and_then(|camera| camera.zoomed(self.zoom))
        .ok_or_else(|| {
            CommandError::new(if self.look_at.is_some() {
                format!(
                    "{}: cannot frame this `box` from this view: seen from there it has no size",
                    self.command
                )
            } else {
                format!("{}: cannot frame the model", self.command)
            })
        })
    }
}

fn view_render(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(
        "view.render",
        params,
        &[
            "path",
            "width",
            "height",
            "view",
            "projection",
            "zoom",
            "box",
            "mode",
            "section",
            "tolerance",
        ],
    )?;
    let width = p.integer("width", 16, 4096)?.unwrap_or(800);
    let height = p.integer("height", 16, 4096)?.unwrap_or(600);
    let framing = Framing::from_params(&p, "view.render")?;
    let (view, orthographic, zoom, look_at) = (
        framing.view,
        framing.orthographic,
        framing.zoom,
        framing.look_at,
    );
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

    // The section, resolved against the scene of the same tolerance: a
    // face's name means the face of that mesh.
    let section = match p.get("section") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let tolerance = match p.number("tolerance")? {
                Some(t) if t > 0.0 => Some(t),
                Some(_) => return fail("view.render: `tolerance` must be positive"),
                None => None,
            };
            let Some(scene) = session.scene(tolerance)? else {
                return fail("view.render: no document is open; run file.new or file.open first");
            };
            let (spec, cap) = crate::section::section_param("view.render", value)?;
            let plane = session
                .section_plane(&scene, &spec)
                .map_err(|e| CommandError::new(format!("view.render: {e}")))?;
            Some((plane, cap))
        }
    };

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
        section: section.map(|(plane, cap)| craft3d_render::Section {
            cap,
            ..plane.section(crate::section::SECTION_CAP_COLOR)
        }),
        ..RenderParams::default()
    };
    let aspect = width as f64 / height as f64;
    let model = all.bbox();
    let framed = look_at.unwrap_or(model);
    let camera = framing.camera(&model, aspect)?;
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
        "projection": if orthographic { "orthographic" } else { "perspective" },
        "zoom": zoom,
        "box": framed.corners().map(|(lo, hi)| json!([vec3(lo), vec3(hi)])),
        "triangles": all.triangles.len(),
        "coverage": frame.coverage(),
        "bodies": legend,
    });
    let Some(map) = result.as_object_mut() else {
        return Ok(result);
    };
    if let Some((plane, _)) = &section {
        map.insert(
            "section".into(),
            json!({
                "point": vec3(plane.origin),
                "normal": vec3(plane.normal),
                "range": plane.range(&model),
            }),
        );
    }
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
