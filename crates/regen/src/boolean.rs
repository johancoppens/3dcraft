//! Booleans in regeneration: the bodies of two earlier features become
//! one. Every face of the result is a face, or a piece of one, of the
//! target or of the tool, and is named after it as a copy: instance 0 is
//! the target, instance 1 the tool (`F7:copy(0)/F2:side(e7)`). A face cut
//! into pieces gives several faces with one name; they lie on one surface.
//! A face the kernel merged from faces of both operands (the flush tops of
//! two united blocks) is named after the first contributor and carries the
//! others' names as aliases. Its edges and vertices are named by the faces
//! they lie on (`F7:between[a,b]`, `F7:corner[a,b,c]`: [`crate::entity`]),
//! and one the target or the tool already had answers to a copy of the
//! name it had there too (`F7:copy(0)/F1:vertex(3)`).

use crate::derive::source_body;
use crate::selfcheck::{crossing_near, roles_of};
use crate::{Contributors, FeatureResult, Kept, Names};
use craft3d_doc::{Boolean, BooleanOp, Document, FeatureId};
use craft3d_math::Tolerance;
use craft3d_topo::Body;

/// What the kernel's boolean made.
pub(crate) struct Combined {
    pub(crate) body: Body,
    /// For every face the operands (0 the target, 1 the tool) and the
    /// faces of them it has a piece of, the one it is named after first.
    pub(crate) contributors: Contributors,
    /// For every edge and vertex the operand and the entity of it that
    /// it is, when it is one.
    pub(crate) edges: Kept,
    pub(crate) vertices: Kept,
}

/// `names` are the names of the target's and the tool's faces, when they
/// have them: a refusal that is about faces of theirs names them so.
pub(crate) fn combine(
    target: &Body,
    tool: &Body,
    op: BooleanOp,
    tolerance: &Tolerance,
    loosest: f64,
    names: [Option<&Names>; 2],
) -> Result<Combined, String> {
    use craft3d_kernel::KernelError;
    use craft3d_kernel::boolean::{
        BooleanOp as Op, BooleanOptions, EdgeSource, FaceSource, Operand, VertexSource,
        boolean_with,
    };
    let op = match op {
        BooleanOp::Union => Op::Union,
        BooleanOp::Subtract => Op::Subtract,
        BooleanOp::Intersect => Op::Intersect,
    };
    let number = |operand: Operand| match operand {
        Operand::A => 0,
        Operand::B => 1,
    };
    // The document's cap: operands with loose entities are worked on at
    // their largest tolerance up to it.
    let options = BooleanOptions {
        loosest: Some(loosest),
        ..BooleanOptions::default()
    };
    let result = boolean_with(target, tool, op, tolerance, &options).map_err(|e| match e {
        KernelError::ZeroThickness { at, faces, .. } => {
            let [a, b] = faces.map(|(operand, face)| {
                let [first, second] = names;
                let names = if number(operand) == 0 { first } else { second };
                names
                    .and_then(|names| names.faces.get(face.0 as usize))
                    .map_or_else(
                        || {
                            let body = if number(operand) == 0 { "target" } else { "tool" };
                            format!("face {} of the {body}", face.0)
                        },
                        ToString::to_string,
                    )
            });
            if target.kind == craft3d_topo::BodyKind::Sheet {
                format!(
                    "the trimmed sheet would touch itself at ({}, {}, {}): its boundary comes back to itself there in a point or along a line, at {a} and {b}, and no sheet can be pinched like that; move the solid by more than the tolerance, or further across the sheet",
                    at.x, at.y, at.z
                )
            } else {
                format!(
                    "the result would have zero thickness at ({}, {}, {}): two of its parts touch there along an edge or in a point, at {a} and {b}, and no solid can be that thin; move them apart by more than the tolerance, or into each other",
                    at.x, at.y, at.z
                )
            }
        }
        other => other.to_string(),
    })?;
    let pair = |source: &FaceSource| (number(source.operand), source.face.0 as usize);
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
    let edges = result
        .edges
        .iter()
        .map(|source| match source {
            EdgeSource::Kept { operand, edge } => Some((number(*operand), edge.0 as usize)),
            EdgeSource::Intersection { .. } => None,
        })
        .collect();
    let vertices = result
        .vertices
        .iter()
        .map(|source| match source {
            VertexSource::Kept { operand, vertex } => Some((number(*operand), vertex.0 as usize)),
            VertexSource::New { .. } => None,
        })
        .collect();
    Ok(Combined {
        body: result.body,
        contributors,
        edges,
        vertices,
    })
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
    let made = combine(
        a,
        b,
        boolean.op,
        &tolerance,
        doc.tolerance.loosest_linear(),
        [target.names.as_deref(), tool.names.as_deref()],
    )?;
    let operand = |operand: u64| {
        let names = if operand == 0 {
            &target.names
        } else {
            &tool.names
        };
        names.as_deref()
    };
    let mut names = Names::copied_faces(feature, &made.contributors, operand)?;
    if let Some(names) = &mut names {
        names.name_entities(
            feature,
            &made.body,
            tolerance.linear,
            &made.edges,
            &made.vertices,
            operand,
        )?;
    }
    not_through_itself(&made, names.as_ref(), tolerance.linear, &[])?;
    Ok((made.body, names))
}

/// Fails when the result of a boolean passes through itself. Its faces
/// are pieces of the faces of two bodies that do not (each was looked at,
/// or cannot, when it was made), so pieces of one operand do not cross
/// each other: what is looked at is where pieces of the two can meet,
/// and the faces merged from both (`selfcheck::crossing_near`). A
/// crossing there is a place where the operands meet that the boolean
/// did not find. `new_from` names operands that were not looked at on
/// their own.
pub(crate) fn not_through_itself(
    made: &Combined,
    names: Option<&Names>,
    linear: f64,
    new_from: &[u64],
) -> Result<(), String> {
    let roles = roles_of(&made.contributors, new_from);
    let faces = names.map_or(&[][..], |names| names.faces.as_slice());
    crossing_near(&made.body, faces, linear, &roles).refused(
        "tool",
        "result",
        "the boolean did not find where the operands meet there, and the result is refused rather than handed on",
    )
}
