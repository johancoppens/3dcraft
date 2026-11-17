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
        boolean_with, sew_sheets,
    };
    let op = match op {
        BooleanOp::Union => Some(Op::Union),
        BooleanOp::Subtract => Some(Op::Subtract),
        BooleanOp::Intersect => Some(Op::Intersect),
        // Not a boolean of the kernel's: the sheets are sewn, not cut.
        BooleanOp::Sew => None,
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
    let result = match op {
        Some(op) => boolean_with(target, tool, op, tolerance, &options),
        None => sew_sheets(target, tool, tolerance),
    };
    // A face of an operand by its name, or by its number and operand.
    let face_name = |operand: Operand, face: craft3d_topo::FaceId| {
        let [first, second] = names;
        let names = if number(operand) == 0 { first } else { second };
        names
            .and_then(|names| names.faces.get(face.0 as usize))
            .map_or_else(
                || {
                    let body = if number(operand) == 0 {
                        "target"
                    } else {
                        "tool"
                    };
                    format!("face {} of the {body}", face.0)
                },
                ToString::to_string,
            )
    };
    let result = result.map_err(|e| match e {
        // A face of more spans than a boolean indexes: which, and how many
        // spans it has.
        KernelError::Unsupported {
            operation,
            reason,
            faces,
        } if reason == craft3d_kernel::boolean::TOO_MANY_SPANS => {
            let named: Vec<String> = faces
                .iter()
                .map(|&(operand, face)| {
                    let body = if number(operand) == 0 { target } else { tool };
                    let spans = body
                        .face(face)
                        .and_then(|f| craft3d_kernel::boolean::span_count(&f.surface));
                    match spans {
                        Some(count) => format!("{} ({count} spans)", face_name(operand, face)),
                        None => face_name(operand, face),
                    }
                })
                .collect();
            format!(
                "{operation} does not support this: a freeform face of more Bézier spans than the {} a boolean indexes{}{}; build it of fewer spans (a grid of fewer points, or the face in parts)",
                craft3d_kernel::boolean::MAX_FREEFORM_SPANS,
                if cfg!(target_arch = "wasm32") {
                    format!(
                        " in the browser (the desktop program takes up to {})",
                        craft3d_kernel::boolean::MAX_FREEFORM_SPANS_DESKTOP
                    )
                } else {
                    String::new()
                },
                if named.is_empty() {
                    String::new()
                } else {
                    format!(": {}", named.join(", "))
                }
            )
        }
        // A face whose spans and index would take more memory than one
        // face may: which, its spans and degrees, and the bytes.
        KernelError::Unsupported {
            operation,
            reason,
            faces,
        } if reason == craft3d_kernel::boolean::TOO_LARGE_INDEX => {
            let named: Vec<String> = faces
                .iter()
                .map(|&(operand, face)| {
                    let body = if number(operand) == 0 { target } else { tool };
                    let surface = body.face(face).map(|f| &f.surface);
                    let spans = surface.and_then(craft3d_kernel::boolean::span_count);
                    let bytes = surface.and_then(craft3d_kernel::boolean::index_bytes);
                    let degrees = match surface {
                        Some(craft3d_geom::Surface::Nurbs(s)) => Some(s.degrees()),
                        _ => None,
                    };
                    match (spans, degrees, bytes) {
                        (Some(spans), Some((p, q)), Some(bytes)) => format!(
                            "{} ({spans} spans of degree {p} × {q}: {} MiB)",
                            face_name(operand, face),
                            bytes.div_ceil(1 << 20)
                        ),
                        _ => face_name(operand, face),
                    }
                })
                .collect();
            format!(
                "{operation} does not support this: a freeform face whose Bézier spans and their index would take more than the {} MiB one face may{}; build it of fewer spans or of a lower degree",
                craft3d_kernel::boolean::MAX_INDEX_BYTES >> 20,
                if named.is_empty() {
                    String::new()
                } else {
                    format!(": {}", named.join(", "))
                }
            )
        }
        KernelError::ZeroThickness { at, faces, .. } => {
            let [a, b] = faces.map(|(operand, face)| face_name(operand, face));
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
        // Faces or points between half and twice the tolerance apart (the
        // Garabit valley's finding 3, 2026-10-21: bars placed by
        // translations rounded to 0.1 mm, whose side faces end up 0.76
        // tolerances apart): what is refused, by the faces' names, and
        // what to do about it.
        KernelError::Unsupported {
            operation,
            reason,
            faces,
        } if reason.ends_with("(at the edge of tolerance)") => {
            let named: Vec<String> = faces
                .iter()
                .map(|&(operand, face)| face_name(operand, face))
                .collect();
            let at = if named.is_empty() {
                String::new()
            } else {
                format!(" ({})", named.join(", "))
            };
            let tol = tolerance.linear;
            format!(
                "{operation} does not support this: {reason}{at}. At the document's tolerance of {tol:e}, faces or points between half of it ({:e}) and twice it ({:e}) apart are neither one place nor two; put them onto each other within half the tolerance (positions with more digits, or placed from the same numbers) or at least twice the tolerance apart",
                0.5 * tol,
                2.0 * tol
            )
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use craft3d_geom::NurbsSurface;
    use craft3d_math::{Frame, Vec3};

    /// A face of more Bézier spans than a boolean indexes is refused with
    /// its name, its count of spans and the limit, before its spans are
    /// made: a flat sheet of 514 × 513 points of degree one, 513 × 512 =
    /// 262 656 spans, against the 262 144 a boolean takes.
    #[test]
    fn a_face_of_more_spans_than_a_boolean_indexes_is_named_with_its_count() {
        let (columns, rows) = (514_usize, 513_usize);
        let knots = |n: usize| {
            let mut k = vec![0.0];
            k.extend((0..n).map(|i| i as f64));
            k.push((n - 1) as f64);
            k
        };
        let mut ctrl = Vec::with_capacity(columns * rows);
        for c in 0..columns {
            for r in 0..rows {
                ctrl.push(Vec3::new(c as f64 * 0.1, r as f64 * 0.1, 1.0));
            }
        }
        let surface = NurbsSurface::new(
            1,
            1,
            knots(columns),
            knots(rows),
            columns,
            rows,
            ctrl,
            vec![1.0; columns * rows],
        )
        .unwrap();
        let spans =
            craft3d_kernel::boolean::span_count(&craft3d_geom::Surface::Nurbs(surface.clone()));
        assert_eq!(spans, Some(262_656));
        let sheet = crate::grid::sheet(surface, rows, columns, false).unwrap();
        let block = craft3d_kernel::primitives::block(&Frame::WORLD, 51.3, 51.2, 4.0).unwrap();
        let tolerance = Tolerance::new(1e-3, 1e-6).unwrap();
        let refused = combine(
            &block,
            &sheet,
            BooleanOp::Intersect,
            &tolerance,
            1.0,
            [None, None],
        )
        .err()
        .expect("refused");
        assert!(
            refused.contains("face 0 of the tool (262656 spans)")
                && refused.contains("than the 262144 a boolean indexes"),
            "{refused}"
        );
    }

    /// A face of fewer spans than that but of a degree whose spans and
    /// index would take more than one face may is refused with its spans,
    /// its degrees, the memory and the budget: degree 15 over 254 × 254
    /// knot spans, some 1.1 GB against 1 GiB.
    #[test]
    fn a_face_whose_index_would_take_too_much_is_named_with_its_bytes() {
        let (degree, spans) = (15_usize, 254_usize);
        let n = spans + degree;
        let mut knots = vec![0.0; degree + 1];
        for k in 1..spans {
            knots.push(k as f64 / spans as f64);
        }
        knots.extend(std::iter::repeat_n(1.0, degree + 1));
        let mut ctrl = Vec::with_capacity(n * n);
        for c in 0..n {
            for r in 0..n {
                ctrl.push(Vec3::new(c as f64 * 0.2, r as f64 * 0.2, 1.0));
            }
        }
        let surface = NurbsSurface::new(
            degree,
            degree,
            knots.clone(),
            knots,
            n,
            n,
            ctrl,
            vec![1.0; n * n],
        )
        .unwrap();
        let bytes =
            craft3d_kernel::boolean::index_bytes(&craft3d_geom::Surface::Nurbs(surface.clone()))
                .unwrap();
        assert!(bytes > craft3d_kernel::boolean::MAX_INDEX_BYTES, "{bytes}");
        let sheet = crate::grid::sheet(surface, n, n, false).unwrap();
        let side = 0.2 * (n - 1) as f64;
        let block = craft3d_kernel::primitives::block(&Frame::WORLD, side, side, 4.0).unwrap();
        let tolerance = Tolerance::new(1e-3, 1e-6).unwrap();
        let refused = combine(
            &block,
            &sheet,
            BooleanOp::Intersect,
            &tolerance,
            1.0,
            [None, None],
        )
        .err()
        .expect("refused");
        let mib = bytes.div_ceil(1 << 20);
        assert!(
            refused.contains(&format!(
                "face 0 of the tool (64516 spans of degree 15 × 15: {mib} MiB)"
            )) && refused.contains("more than the 1024 MiB one face may"),
            "{refused}"
        );
    }
}
