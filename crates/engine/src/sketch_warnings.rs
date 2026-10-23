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
/// the line, and horizontal or vertical between its two ends; and how
/// many different directions they ask for (the same kind towards the
/// same line with the same value is one direction, said twice).
fn direction_constraints(
    sketch: &Sketch,
    line: EntityId,
    start: EntityId,
    end: EntityId,
) -> (Vec<String>, usize) {
    let ends = |a: EntityId, b: Option<EntityId>| {
        b.is_some_and(|b| (a == start && b == end) || (a == end && b == start))
    };
    let other = |a: EntityId, b: EntityId| if a == line { b } else { a };
    let mut ids = Vec::new();
    let mut directions: Vec<String> = Vec::new();
    for c in &sketch.constraints {
        let direction = match &c.kind {
            ConstraintKind::Horizontal { a, b } if (*a == line && b.is_none()) || ends(*a, *b) => {
                "horizontal".to_string()
            }
            ConstraintKind::Vertical { a, b } if (*a == line && b.is_none()) || ends(*a, *b) => {
                "vertical".to_string()
            }
            ConstraintKind::Parallel { a, b } if *a == line || *b == line => {
                format!("parallel {}", other(*a, *b))
            }
            ConstraintKind::Perpendicular { a, b } if *a == line || *b == line => {
                format!("perpendicular {}", other(*a, *b))
            }
            ConstraintKind::Angle { a, b, value } if *a == line || *b == line => {
                format!("angle {} {}", other(*a, *b), value.source())
            }
            _ => continue,
        };
        ids.push(c.id.to_string());
        if !directions.contains(&direction) {
            directions.push(direction);
        }
    }
    (ids, directions.len())
}

/// The lines of `sketch` whose two ends the solve put on one point while
/// constraints give them a direction: `[{"line", "constraints",
/// "contradiction", "problem"}]`. Such a line has no direction, and the
/// solver reports what is asked of it as holding because a point has
/// every direction. With two directions or more (horizontal and
/// vertical; a direction and a perpendicular or an angle) they contradict
/// each other: `contradiction` true. With one, the line may merely have
/// been drawn or constrained to no length: false, a warning at most.
/// `tolerance` is the document's linear one.
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
        if length.is_nan() || length > tolerance {
            continue;
        }
        let (constraints, directions) = direction_constraints(sketch, entity.id, start, end);
        if constraints.is_empty() {
            continue;
        }
        let contradiction = directions >= 2;
        let problem = if contradiction {
            format!(
                "line {} has shrunk to a point: {} give it directions it can only take with no length; delete or change one of them (sketch.delete, sketch.edit)",
                entity.id,
                constraints.join(", "),
            )
        } else {
            format!(
                "line {} has no length: the direction {} gives it does not show; give it a length (a distance) if it is to have one",
                entity.id,
                constraints.join(", "),
            )
        };
        out.push(json!({
            "line": entity.id.to_string(),
            "constraints": constraints,
            "contradiction": contradiction,
            "problem": problem,
        }));
    }
    out
}

/// For `document.check`: per sketch that solved, its lines collapsed
/// under two directions as problems (a contradiction), one under a single
/// direction as a warning (`kind: "collapsedLine"`), and one warning for
/// all the sketches with degrees of freedom left (`kind:
/// "underConstrained"`, with `sketches: [{feature, name, dof}]`): an
/// under-constrained sketch keeps the coordinates it was drawn at, which
/// a later edit may move.
pub(crate) fn check_sketches(
    doc: &Document,
    regen: &Regen,
    only: Option<FeatureId>,
    problems: &mut Vec<Value>,
    warnings: &mut Vec<Value>,
) {
    let mut free = Vec::new();
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
            let contradiction = collapsed
                .get("contradiction")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if contradiction {
                problems.push(json!({
                    "feature": outcome.id.to_string(),
                    "name": feature.name,
                    "line": collapsed.get("line"),
                    "constraints": collapsed.get("constraints"),
                    "problem": collapsed.get("problem"),
                }));
            } else {
                warnings.push(json!({
                    "kind": "collapsedLine",
                    "feature": outcome.id.to_string(),
                    "name": feature.name,
                    "line": collapsed.get("line"),
                    "constraints": collapsed.get("constraints"),
                    "warning": collapsed.get("problem"),
                }));
            }
        }
        if let Some(dof) = solved.dof.filter(|d| *d > 0) {
            free.push(json!({
                "feature": outcome.id.to_string(),
                "name": feature.name,
                "dof": dof,
            }));
        }
    }
    if !free.is_empty() {
        let count = free.len();
        warnings.push(json!({
            "kind": "underConstrained",
            "sketches": free,
            "warning": format!(
                "{count} sketch{} under-constrained (degrees of freedom left, per sketch under sketches): {} the coordinates {} drawn at, which a later edit may move; sketch.constrain, or coordinates as expressions, hold them",
                if count == 1 { " is" } else { "es are" },
                if count == 1 { "it keeps" } else { "they keep" },
                if count == 1 { "it was" } else { "they were" },
            ),
        }));
    }
}
