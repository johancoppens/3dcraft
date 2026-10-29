//! `file.export` of an assembly as STEP with its structure: each distinct
//! body of a configuration once, as a part, placed by every instance that
//! shows it (`craft3d_io::step_assembly`). Subassemblies are written
//! flat: their parts placed where the subassembly puts them.

use crate::asm::{Built, InstanceState, Placed, bodies_of};
use crate::commands::{Run, fail};
use crate::{CommandError, Session};
use craft3d_doc::{AngleUnit, FeatureId, LengthUnit};
use craft3d_io::{StepBody, StepOccurrence, StepOptions};
use craft3d_math::Transform;
use craft3d_topo::Body;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// A part placed: its key (configuration and body), its name, its body
/// in its own coordinates, where it is placed and the occurrence's name.
struct Leaf {
    key: String,
    name: String,
    body: Arc<Body>,
    pose: Transform,
    occurrence: String,
}

fn leaves_of(
    st: &InstanceState,
    pose: &Transform,
    prefix: &str,
    depth: usize,
    out: &mut Vec<Leaf>,
) {
    let Some(built) = &st.built else {
        return;
    };
    let here = if prefix.is_empty() {
        st.name.clone()
    } else {
        format!("{prefix}/{}", st.name)
    };
    match built.as_ref() {
        Built::Part { .. } => {
            let bodies = bodies_of(st);
            let several = bodies.len() > 1;
            for (name, body, _, _) in bodies {
                out.push(Leaf {
                    key: format!("{}#{name}", st.config.clone().unwrap_or_default()),
                    name: name.clone(),
                    body,
                    pose: *pose,
                    occurrence: if several {
                        format!("{here}/{name}")
                    } else {
                        here.clone()
                    },
                });
            }
        }
        Built::Assembly(placed) => {
            if depth > crate::asm::MAX_DEPTH {
                return;
            }
            for inner in placed
                .instances
                .iter()
                .filter(|i| i.error.is_none() && !i.flexed && shows(placed, i))
            {
                leaves_of(
                    inner,
                    &inner.pose.then(pose),
                    &here,
                    depth.saturating_add(1),
                    out,
                );
            }
        }
    }
}

/// True when the instance shows a body in `placed`: an invisible one is
/// left out, as the mesh exports and the scene leave it out.
fn shows(placed: &Placed, st: &InstanceState) -> bool {
    st.bodies.iter().any(|b| !placed.doc.is_hidden(*b))
}

fn leaves(placed: &Placed, only: Option<&[FeatureId]>) -> Vec<Leaf> {
    let mut out = Vec::new();
    // A flexible subassembly's bodies are its instances'.
    for st in placed
        .instances
        .iter()
        .filter(|i| i.error.is_none() && !i.flexed && shows(placed, i))
    {
        if only.is_some_and(|only| !st.bodies.iter().any(|b| only.contains(b))) {
            continue;
        }
        leaves_of(st, &st.pose, "", 0, &mut out);
    }
    out
}

pub(crate) fn step(
    session: &mut Session,
    path: &str,
    resolved: &Path,
    only: Option<&[FeatureId]>,
) -> Run {
    let placed = session.placed()?;
    let doc = Arc::clone(session.document()?.history.current());
    let leaves = leaves(&placed, only);
    if leaves.is_empty() {
        return fail(
            "file.export: the assembly has no placed bodies; see assembly.inspect for instances that failed",
        );
    }
    // Each distinct part once, in the order it is first placed.
    let mut parts: Vec<&Leaf> = Vec::new();
    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    let mut occurrences = Vec::with_capacity(leaves.len());
    for leaf in &leaves {
        let part = match index.get(leaf.key.as_str()) {
            Some(&k) => k,
            None => {
                index.insert(leaf.key.as_str(), parts.len());
                parts.push(leaf);
                parts.len().saturating_sub(1)
            }
        };
        occurrences.push(StepOccurrence {
            part,
            pose: leaf.pose,
            name: leaf.occurrence.clone(),
        });
    }
    let bodies: Vec<StepBody<'_>> = parts
        .iter()
        .map(|leaf| StepBody::new(&leaf.name, &leaf.body))
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
    let name = resolved
        .file_stem()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("Assembly")
        .to_string();
    let (bytes, summary) = craft3d_io::step_assembly(&name, &bodies, &occurrences, &options)
        .map_err(|e| CommandError::new(format!("file.export: {e}")))?;
    let size = bytes.len();
    session.write_resolved(resolved, bytes)?;
    Ok(json!({
        "path": path,
        "format": "step",
        "schema": craft3d_io::step::SCHEMA,
        "bytes": size,
        "parts": summary.bodies,
        "occurrences": occurrences.len(),
        "bodies": occurrences.len(),
        "solids": summary.solids,
        "faces": summary.faces,
        "entities": summary.instances,
        "unit": doc.units.length.symbol(),
        "uncertainty": tolerance.linear,
    }))
}
