//! The STEP side of `file.export` (the command itself is in
//! `commands.rs`): every body of the active document, or those asked
//! for, as one STEP file.

use crate::commands::{Run, fail};
use crate::{CommandError, Session};
use craft3d_doc::{AngleUnit, FeatureId, LengthUnit};
use craft3d_io::{StepBody, StepOptions};
use serde_json::json;
use std::path::Path;

/// Whether `file.export` should write STEP: by `format`, or else by the
/// extension of `path`.
pub(crate) fn wanted(format: Option<&str>, path: &str) -> bool {
    match format {
        Some(format) => matches!(format, "step" | "stp"),
        None => Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(craft3d_io::step::is_step_extension),
    }
}

/// Writes every body of the active document to `resolved` as STEP, or
/// the bodies of the features of `only` when given. `path` is the path as
/// the request gave it, for the result.
pub(crate) fn export(
    session: &mut Session,
    path: &str,
    resolved: &Path,
    only: Option<&[FeatureId]>,
) -> Run {
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = &open.bodies_doc();
    // Each body with its name and colours (`appearance`), and the share of
    // light each lets through (layer 3's transmission).
    struct Named<'b> {
        name: String,
        body: &'b craft3d_topo::Body,
        color: Option<[u8; 3]>,
        faces: Vec<Option<[u8; 3]>>,
        clear: Option<f64>,
        clear_faces: Vec<Option<f64>>,
    }
    let transmission =
        |a: &craft3d_doc::Appearance| Some(a.optics.transmission).filter(|t| *t > 0.0);
    let named: Vec<Named<'_>> = regen
        .bodies()
        .filter(|(id, _)| only.is_none_or(|only| only.contains(id)))
        .map(|(id, body)| {
            let name = doc
                .feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone());
            let own = crate::appearance::body_appearance(doc, &regen, id);
            let looks = crate::appearance::face_appearances(doc, &regen, id, body.faces.len());
            Named {
                name,
                body,
                color: own.map(|a| a.color),
                clear: own.as_ref().and_then(transmission),
                faces: looks.iter().map(|a| a.map(|a| a.color)).collect(),
                clear_faces: looks
                    .iter()
                    .map(|a| a.as_ref().and_then(transmission))
                    .collect(),
            }
        })
        .collect();
    if named.is_empty() {
        return fail(
            "file.export: there are no bodies; add a feature, or check document.inspect for failed features",
        );
    }
    let bodies: Vec<StepBody<'_>> = named
        .iter()
        .map(|n| StepBody {
            color: n.color,
            face_colors: &n.faces,
            transparency: n.clear,
            face_transparency: &n.clear_faces,
            ..StepBody::new(&n.name, n.body)
        })
        .collect();
    let tolerance = craft3d_math::Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or_else(|| {
            CommandError::new("file.export: the document's tolerance is not positive and finite")
        })?;
    let options = StepOptions {
        length_unit: match doc.units.length {
            LengthUnit::Micrometre => craft3d_meshio::Unit::Micron,
            LengthUnit::Millimetre => craft3d_meshio::Unit::Millimeter,
            LengthUnit::Centimetre => craft3d_meshio::Unit::Centimeter,
            LengthUnit::Metre => craft3d_meshio::Unit::Meter,
            LengthUnit::Inch => craft3d_meshio::Unit::Inch,
            LengthUnit::Foot => craft3d_meshio::Unit::Foot,
        },
        angle_unit: match doc.units.angle {
            AngleUnit::Degree => craft3d_io::AngleUnit::Degree,
            AngleUnit::Radian => craft3d_io::AngleUnit::Radian,
        },
        tolerance,
        file_name: resolved
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        ..StepOptions::default()
    };
    let (bytes, summary) = craft3d_io::export_step(&bodies, &options)
        .map_err(|e| CommandError::new(format!("file.export: {e}")))?;
    let size = bytes.len();
    let unit = doc.units.length.symbol();
    session.write_resolved(resolved, bytes)?;
    Ok(json!({
        "path": path,
        "format": "step",
        "schema": craft3d_io::step::SCHEMA,
        "bytes": size,
        "bodies": summary.bodies,
        // Lumps: a body in several pieces is several solids.
        "solids": summary.solids,
        "cavities": summary.voids,
        "shells": summary.shells,
        "faces": summary.faces,
        "edges": summary.edges,
        "vertices": summary.vertices,
        "entities": summary.instances,
        "unit": unit,
        // The document's tolerance, written as the file's uncertainty.
        "uncertainty": tolerance.linear,
    }))
}
