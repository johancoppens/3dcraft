//! Booleans in regeneration: the bodies of two earlier features become
//! one. Every face of the result is a face, or a piece of one, of the
//! target or of the tool, and is named after it as a copy: instance 0 is
//! the target, instance 1 the tool (`F7:copy(0)/F2:side(e7)`). A face cut
//! into pieces gives several faces with one name; they lie on one surface.
//! A face the kernel merged from faces of both operands (the flush tops of
//! two united blocks) is named after the first contributor and carries the
//! others' names as aliases. Edges and vertices of a boolean have no names
//! yet.

use crate::derive::source_body;
use crate::{Contributors, FeatureResult, Names};
use craft3d_doc::{Boolean, BooleanOp, Document, FeatureId};
use craft3d_math::Tolerance;
use craft3d_topo::Body;

/// The kernel's boolean: the body, and for every face of it the operands
/// (0 the target, 1 the tool) and the faces of them it has a piece of, the
/// one it is named after first.
fn combine(
    target: &Body,
    tool: &Body,
    op: BooleanOp,
    tolerance: &Tolerance,
) -> Result<(Body, Contributors), String> {
    use craft3d_kernel::boolean::{BooleanOp as Op, FaceSource, Operand, boolean};
    let op = match op {
        BooleanOp::Union => Op::Union,
        BooleanOp::Subtract => Op::Subtract,
        BooleanOp::Intersect => Op::Intersect,
    };
    let result = boolean(target, tool, op, tolerance).map_err(|e| e.to_string())?;
    let pair = |source: &FaceSource| {
        let operand = match source.operand {
            Operand::A => 0,
            Operand::B => 1,
        };
        (operand, source.face.0 as usize)
    };
    let mut contributors = Contributors::with_capacity(result.faces.len());
    for (index, source) in result.faces.iter().enumerate() {
        // The kernel reports the first contributor as the source and lists
        // every contributor under `merged`; the source is put first here
        // whatever the list says, so the face keeps the name it has
        // without merging.
        let mut sources = vec![pair(source)];
        for other in result.merged.get(index).into_iter().flatten() {
            let other = pair(other);
            if !sources.contains(&other) {
                sources.push(other);
            }
        }
        contributors.push(sources);
    }
    Ok((result.body, contributors))
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
    let (body, contributors) = combine(a, b, boolean.op, &tolerance)?;
    let names = Names::copied_faces(feature, &contributors, |operand, face| {
        let names = if operand == 0 {
            &target.names
        } else {
            &tool.names
        };
        names.as_ref().and_then(|n| n.faces.get(face))
    })?;
    Ok((body, names))
}
