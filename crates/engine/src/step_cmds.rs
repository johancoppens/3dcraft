//! The STEP side of `file.export` (the command itself is in
//! `commands.rs`): every body of the active document as one STEP file.

use crate::commands::{Run, fail};
use crate::{CommandError, Session};
use craft3d_doc::{AngleUnit, LengthUnit};
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

/// Writes every body of the active document to `resolved` as STEP.
/// `path` is the path as the request gave it, for the result.
pub(crate) fn export(session: &mut Session, path: &str, resolved: &Path) -> Run {
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let named: Vec<(String, &craft3d_topo::Body)> = regen
        .bodies()
        .map(|(id, body)| {
            let name = doc
                .feature(id)
                .map_or_else(|| id.to_string(), |f| f.name.clone());
            (name, &**body)
        })
        .collect();
    if named.is_empty() {
        return fail(
            "file.export: there are no bodies; add a feature, or check document.inspect for failed features",
        );
    }
    let bodies: Vec<StepBody<'_>> = named
        .iter()
        .map(|(name, body)| StepBody { name, body })
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
    crate::workspace::write_atomic(resolved, &bytes)?;
    Ok(json!({
        "path": path,
        "format": "step",
        "schema": craft3d_io::step::SCHEMA,
        "bytes": bytes.len(),
        "bodies": summary.bodies,
        // Lumps: a body in several pieces is several solids.
        "solids": summary.solids,
        "cavities": summary.voids,
        "shells": summary.shells,
        "faces": summary.faces,
        "edges": summary.edges,
        "vertices": summary.vertices,
        "entities": summary.instances,
        "unit": doc.units.length.symbol(),
        // The document's tolerance, written as the file's uncertainty.
        "uncertainty": tolerance.linear,
    }))
}
