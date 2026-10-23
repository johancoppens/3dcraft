//! What a solved sketch does not say by its statuses (R1, the second field
//! test): a line the solver met two directions on by shrinking it to a
//! point (`horizontal` and `vertical` on one line), and degrees of freedom
//! left. Diagnosis only: the solve, its statuses and what is built from
//! the sketch stay as they are.

use craft3d_doc::sketch::{ConstraintKind, EntityId, Geometry, Sketch};
use craft3d_doc::{Document, FeatureId, FeatureKind};
use craft3d_regen::{Regen, SketchResult, Status};
use serde_json::{Value, json};

/// The constraints that give line `line` (from `start` to `end`) a
/// direction: horizontal, vertical, parallel, perpendicular and angle on
/// the line, and horizontal or vertical between its two ends.
fn direction_constraints(
    sketch: &Sketch,
    line: EntityId,
    start: EntityId,
    end: EntityId,
) -> Vec<String> {
    let ends = |a: EntityId, b: Option<EntityId>| {
        b.is_some_and(|b| (a == start && b == end) || (a == end && b == start))
    };
    sketch
        .constraints
        .iter()
        .filter(|c| match c.kind {
            ConstraintKind::Horizontal { a, b } | ConstraintKind::Vertical { a, b } => {
                (a == line && b.is_none()) || ends(a, b)
            }
            ConstraintKind::Parallel { a, b }
            | ConstraintKind::Perpendicular { a, b }
            | ConstraintKind::Angle { a, b, .. } => a == line || b == line,
            _ => false,
        })
        .map(|c| c.id.to_string())
        .collect()
}

/// The lines of `sketch` whose two ends the solve put on one point while
/// constraints give them a direction: `[{"line", "constraints",
/// "problem"}]`. Such a line has no direction, so the directions asked of
/// it contradict each other (or ask for a length the others take away);
/// the solver reports them as holding because a point has every
/// direction. `tolerance` is the document's linear one.
pub(crate) fn collapsed_lines(
    sketch: &Sketch,
    solved: &SketchResult,
    tolerance: f64,
) -> Vec<Value> {
    let mut out = Vec::new();
    for entity in &sketch.entities {
        let Geometry::Line { start, end } = entity.geometry else {
            continue;
        };
        let (Some((ax, ay)), Some((bx, by))) = (solved.point(start), solved.point(end)) else {
            continue;
        };
        let length = (bx - ax).hypot(by - ay);
        if !(length <= tolerance) {
            continue;
        }
        let constraints = direction_constraints(sketch, entity.id, start, end);
        if constraints.is_empty() {
            continue;
        }
        out.push(json!({
            "line": entity.id.to_string(),
            "constraints": constraints,
            "problem": format!(
                "line {} has shrunk to a point: {} give it directions it can only take with no length; delete or change one of them (sketch.delete, sketch.edit)",
                entity.id,
                constraints.join(", "),
            ),
        }));
    }
    out
}

/// For `document.check`: per sketch that solved, its collapsed lines as
/// problems (a contradiction) and its degrees of freedom left as a
/// warning (an under-constrained sketch keeps the coordinates it was
/// drawn at, which a later edit may move).
pub(crate) fn check_sketches(
    doc: &Document,
    regen: &Regen,
    only: Option<FeatureId>,
    problems: &mut Vec<Value>,
    warnings: &mut Vec<Value>,
) {
    for outcome in &regen.features {
        if outcome.status != Status::Ok || only.is_some_and(|id| id != outcome.id) {
            continue;
        }
        let (Some(solved), Some(feature)) = (outcome.sketch.as_deref(), doc.feature(outcome.id))
        else {
            continue;
        };
        let FeatureKind::Sketch(sketch) = &feature.kind else {
            continue;
        };
        for collapsed in collapsed_lines(sketch, solved, doc.tolerance.linear) {
            problems.push(json!({
                "feature": outcome.id.to_string(),
                "name": feature.name,
                "line": collapsed.get("line"),
                "constraints": collapsed.get("constraints"),
                "problem": collapsed.get("problem"),
            }));
        }
        if let Some(dof) = solved.dof.filter(|d| *d > 0) {
            warnings.push(json!({
                "feature": outcome.id.to_string(),
                "name": feature.name,
                "dof": dof,
                "warning": format!(
                    "the sketch is under-constrained: {dof} degree{} of freedom left; it keeps the coordinates it was drawn at, which a later edit may move (sketch.constrain, or coordinates as expressions, hold it)",
                    if dof == 1 { "" } else { "s" }
                ),
            }));
        }
    }
}
