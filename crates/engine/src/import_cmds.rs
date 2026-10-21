//! STEP files read into the document (docs/design/step-import.md).
//!
//! `file.import` adds one `import` feature per solid or sheet of the file,
//! all in one undo step. Each holds the file's bytes (shared, not copied:
//! one `Blob` for all of them and for every undo step) and its solid's
//! number, so the document regenerates the bodies without the file on
//! disk, and a saved `.3dcraft` holds the file once. Every solid is read
//! before anything is added: a file with a solid that cannot be read, or
//! that would need a tolerance per edge, changes nothing and says why.

use crate::commands::{Run, edit, fail, feature_result, insert_feature, spec};
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Blob, DocError, FeatureKind, Import};
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("file.import", "Import", r#"{"path": string, "names": [string] = automatic}"#,
        "Reads a STEP file (.step, .stp; AP203, AP214 or AP242 B-rep) from the read root into the active document: one import feature per placed solid (MANIFOLD_SOLID_BREP, BREP_WITH_VOIDS with its cavities) or sheet (SHELL_BASED_SURFACE_MODEL) of the file, as one undo step: an assembly (NEXT_ASSEMBLY_USAGE_OCCURRENCE with its transformations, MAPPED_ITEM) gives one feature per placement, moved where the assembly puts it, named after its place (bracket/bolt#2, also in the result's `instance`); the solids' colours (STYLED_ITEM) become the bodies' appearances. The document keeps the file's bytes (saved once in the .3dcraft file), so it regenerates without the file. Lengths are converted from the file's unit to the document's; the file's uncertainty is reported and used to join vertices the file repeats (topology only: geometry is never moved, the largest gap joined is reported). Faces, edges and vertices are named by their order in the file (F5:face(0), F5:edge(3)), so importing the same file again gives the same names. Read: planes, cylinders, cones, spheres, tori, B-spline surfaces (rational, uniform, quasi-uniform and Bézier too), surfaces of revolution and linear extrusion of any of the curves read (exact B-spline surfaces where no elementary kind is), offsets of the elementary surfaces, lines, circles, ellipses, polylines, B-spline curves. Refused, naming the entity: other kinds, placements that scale or mirror, assemblies that contain themselves, files without a length unit, and a solid whose edges or vertices lie off their faces by more than the document's cap on an entity's tolerance. The features take the names the file gives its solids where the document can, or names gives them, one per solid.", doc: true, file_import),
];

#[cfg(not(target_arch = "wasm32"))]
fn file_import(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "file.import";
    let p = crate::params::Params::new(ID, params, &["path", "names"])?;
    let path = p.required_str("path")?;
    let given: Option<Vec<String>> = match p.get("names") {
        None => None,
        Some(Value::Array(items)) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                let Some(text) = item.as_str() else {
                    return fail(format!("{ID}: `names` must be a list of strings"));
                };
                names.push(text.to_string());
            }
            Some(names)
        }
        Some(_) => return fail(format!("{ID}: `names` must be a list of strings")),
    };
    let resolved = session.workspace.resolve_read(path)?;
    let size = std::fs::metadata(&resolved)
        .map_err(|e| CommandError::new(format!("{ID}: cannot read `{path}`: {e}")))?
        .len();
    if size > craft3d_stepread::MAX_IMPORT_BYTES as u64 {
        return fail(format!(
            "{ID}: `{path}` is {size} bytes; at most {} are imported",
            craft3d_stepread::MAX_IMPORT_BYTES
        ));
    }
    let bytes = std::fs::read(&resolved)
        .map_err(|e| CommandError::new(format!("{ID}: cannot read `{path}`: {e}")))?;
    let file_name = resolved
        .file_name()
        .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    import_bytes(session, ID, bytes, &file_name, given)
}

#[cfg(target_arch = "wasm32")]
fn file_import(_: &mut Session, _: &Value) -> Run {
    fail("file.import: files are not available in this build")
}

/// Adds the import features of a file's bytes, after reading every solid
/// of it with the document's units and tolerance. (In the browser build
/// there are no files to read yet.)
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn import_bytes(
    session: &mut Session,
    id: &'static str,
    bytes: Vec<u8>,
    file_name: &str,
    given: Option<Vec<String>>,
) -> Run {
    let blob = Blob::new(bytes);
    let parsed = craft3d_stepread::parse_shared(blob.bytes(), &blob.sha256())
        .map_err(|e| CommandError::new(format!("{id}: `{file_name}`: {e}")))?;
    let items = parsed.items();
    let placed = parsed.placed();
    if items.is_empty() || placed.is_empty() {
        return fail(format!(
            "{id}: `{file_name}` has no solids or sheets (MANIFOLD_SOLID_BREP, BREP_WITH_VOIDS, SHELL_BASED_SURFACE_MODEL)"
        ));
    }
    if let Some(names) = &given
        && names.len() != placed.len()
    {
        return fail(format!(
            "{id}: `names` has {} names for the {} placed solids and sheets of `{file_name}`",
            names.len(),
            placed.len()
        ));
    }
    let options = {
        let doc = session.document()?.history.current();
        let tolerance = craft3d_math::Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
            .ok_or_else(|| {
            CommandError::new(format!(
                "{id}: the document's tolerance is not positive and finite"
            ))
        })?;
        craft3d_stepread::Options {
            metres_per_unit: craft3d_regen::metres_per_unit(doc.units.length),
            tolerance,
            loosest: doc.tolerance.loosest_linear(),
        }
    };
    let mut reports = Vec::with_capacity(placed.len());
    let mut colors = Vec::with_capacity(placed.len());
    let mut step_names: Vec<String> = Vec::with_capacity(placed.len());
    for (index, at) in placed.iter().enumerate() {
        let Some(item) = items.get(at.item) else {
            return fail(format!(
                "{id}: `{file_name}`: placed solid {index} has no solid"
            ));
        };
        let read = parsed.build(index, &options).map_err(|e| {
            CommandError::new(format!(
                "{id}: `{file_name}`, solid {index} (#{}{}): {e}",
                item.id,
                if at.path.is_empty() {
                    String::new()
                } else {
                    format!(" at {}", at.path)
                }
            ))
        })?;
        colors.push(read.color);
        // A placed solid is named after its place in the assembly (the
        // last step of its path), else after the solid.
        step_names.push(
            at.path
                .rsplit('/')
                .next()
                .filter(|leaf| !leaf.is_empty())
                .map_or_else(|| item.name.clone(), str::to_string),
        );
        let r = &read.report;
        reports.push(json!({
            "solid": index,
            "step": format!("#{}", item.id),
            "stepName": item.name,
            "instance": at.path,
            "color": read.color.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")),
            "kind": match item.kind {
                craft3d_stepread::ItemKind::Solid => "solid",
                craft3d_stepread::ItemKind::Sheet => "sheet",
            },
            "fileUnit": r.length_unit,
            "scale": r.scale,
            "uncertainty": r.uncertainty,
            "mergedVertices": r.merged_vertices,
            "vertexGap": r.vertex_gap,
            "mergedEdges": r.merged_edges,
            "edgeGap": r.edge_gap,
            "turnedShells": r.turned_shells,
            "looseEdges": r.loose_edges,
            "looseVertices": r.loose_vertices,
            "unprovenEdges": r.unproven_edges,
            "largestTolerance": r.largest_tolerance,
        }));
    }
    let count = placed.len();
    // The document must still save to a file that opens again.
    {
        let doc = session.document()?.history.current();
        let size = craft3d_format::size_with(doc, &blob, count)
            .map_err(|e| CommandError::new(format!("{id}: {e}")))?;
        if size > craft3d_format::MAX_FILE_BYTES {
            return fail(format!(
                "{id}: with `{file_name}` the document would be a {size}-byte .3dcraft file, and at most {} bytes are opened again; it is not imported",
                craft3d_format::MAX_FILE_BYTES
            ));
        }
    }
    let label = format!("Import {file_name}");
    let ids = edit(session, id, &label, |doc| {
        let mut ids = Vec::with_capacity(count);
        for index in 0..count {
            let kind = FeatureKind::Import(Import {
                file_name: file_name.to_string(),
                solid: u32::try_from(index).unwrap_or(u32::MAX),
                data: blob.clone(),
            });
            let added = match given.as_ref().and_then(|names| names.get(index)) {
                Some(name) => insert_feature(doc, Some(name), kind)?,
                // The name the file gives the solid, when the document
                // can take it; else the automatic one.
                None => match step_names.get(index).filter(|n| !n.is_empty()) {
                    Some(name) => match insert_feature(doc, Some(name), kind.clone()) {
                        Ok(added) => added,
                        Err(
                            DocError::DuplicateFeatureName(_) | DocError::InvalidFeatureName(_),
                        ) => insert_feature(doc, None, kind)?,
                        Err(other) => return Err(other),
                    },
                    None => insert_feature(doc, None, kind)?,
                },
            };
            // The file's colour of the solid, as the body's appearance
            // (in the same undo step).
            if let Some(Some(color)) = colors.get(index) {
                doc.set_appearance(added, Some(craft3d_doc::Appearance::color(*color)))?;
            }
            ids.push(added);
        }
        Ok(ids)
    })?;
    let mut bodies = Vec::with_capacity(ids.len());
    for (feature, report) in ids.iter().zip(reports) {
        let mut result = feature_result(session, *feature)?;
        if let (Some(map), Value::Object(extra)) = (result.as_object_mut(), report) {
            map.insert("import".into(), Value::Object(extra));
        }
        bodies.push(result);
    }
    Ok(json!({
        "file": file_name,
        "bytes": blob.len(),
        "sha256": blob.sha256(),
        "bodies": bodies,
    }))
}
