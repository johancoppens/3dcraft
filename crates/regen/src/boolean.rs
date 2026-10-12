//! Booleans in regeneration: the bodies of two earlier features become
//! one. Every face of the result is a face, or a piece of one, of the
//! target or of the tool, and is named after it as a copy: instance 0 is
//! the target, instance 1 the tool (`F7:copy(0)/F2:side(e7)`). A face cut
//! into pieces gives several faces with one name; they lie on one surface.
//! Edges and vertices of a boolean have no names yet.

use crate::derive::source_body;
use crate::{FeatureResult, Names};
use craft3d_doc::{Boolean, BooleanOp, Document, FeatureId, MAX_NAME_DEPTH, PersistentName};
use craft3d_math::Tolerance;
use craft3d_topo::Body;

/// The kernel's boolean: the body, and for every face of it the operand
/// (0 the target, 1 the tool) and the face of that operand it came from.
fn combine(
    target: &Body,
    tool: &Body,
    op: BooleanOp,
    tolerance: &Tolerance,
) -> Result<(Body, Vec<(u64, usize)>), String> {
    // TEMPORARY, see pending.rs.
    crate::pending::boolean(target, tool, op, tolerance)
}

pub(crate) fn build(
    doc: &Document,
    feature: FeatureId,
    boolean: &Boolean,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    if boolean.target == boolean.tool {
        return Err("tool: a body cannot be combined with itself".to_string());
    }
    let target = source_body(doc, earlier, boolean.target, "target")?;
    let tool = source_body(doc, earlier, boolean.tool, "tool")?;
    let (Some(a), Some(b)) = (&target.body, &tool.body) else {
        return Err("target: it has no body".to_string());
    };
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let (body, sources) = combine(a, b, boolean.op, &tolerance)?;
    // A face is named only when the operand it came from names its faces.
    let mut faces = Vec::with_capacity(sources.len());
    for (operand, face) in sources {
        let names = if operand == 0 {
            &target.names
        } else {
            &tool.names
        };
        let Some(name) = names.as_ref().and_then(|n| n.faces.get(face)) else {
            return Ok((body, None));
        };
        if name.depth() >= MAX_NAME_DEPTH {
            return Err(format!(
                "more than {MAX_NAME_DEPTH} features in a row each make their body from the one before"
            ));
        }
        faces.push(PersistentName::copy(feature, operand, name.clone()));
    }
    Ok((
        body,
        Some(Names {
            faces,
            edges: Vec::new(),
            vertices: Vec::new(),
        }),
    ))
}
