//! Extrusions in regeneration: the solved sketch becomes plain curves, the
//! kernel finds the closed profiles and sweeps them.

use crate::{FeatureResult, Status, length};
use craft3d_doc::{Document, Extrude, FeatureId, FeatureKind, Geometry, NameRole, PersistentName};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::{Origin, Origins, Role, extrude};
use craft3d_kernel::profile::{Curve2, SketchCurve, detect_profiles};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::Body;

/// The persistent name of an entity an extrusion made: the feature, the
/// part the entity plays and the sketch entities it came from, such as
/// `F2:side(e7)` or `F2:lateralEdge(e7,e8)`. It does not depend on the order
/// or number of entities in the body (docs/architecture.md §4.1).
pub fn name_of(feature: FeatureId, origin: &Origin) -> PersistentName {
    PersistentName {
        feature,
        role: match origin.role {
            Role::StartCap => NameRole::StartCap,
            Role::EndCap => NameRole::EndCap,
            Role::Side => NameRole::Side,
            Role::StartEdge => NameRole::StartEdge,
            Role::EndEdge => NameRole::EndEdge,
            Role::LateralEdge => NameRole::LateralEdge,
            Role::StartVertex => NameRole::StartVertex,
            Role::EndVertex => NameRole::EndVertex,
        },
        sources: origin.sources.clone(),
    }
}

/// Why a sketch has no usable profile, with its curves called what the
/// sketch calls them (`e7`).
fn profile_error(error: &KernelError) -> String {
    let list = |curves: &[u64]| {
        curves
            .iter()
            .map(|id| format!("e{id}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match error {
        KernelError::NoProfile => "there is no closed profile".to_string(),
        KernelError::OpenProfile { curves } => format!(
            "the profile is not closed: an end of {} meets nothing",
            list(curves)
        ),
        KernelError::BranchingProfile { curves } => format!(
            "more than two curves meet in one point ({}); a profile is a simple loop",
            list(curves)
        ),
        KernelError::CrossingCurves { curves } => format!(
            "{} cross or touch each other; the loops of a profile must stay apart",
            list(curves)
        ),
        KernelError::ZeroAreaLoop { curves } => {
            format!("the loop of {} encloses no area", list(curves))
        }
        KernelError::DegenerateCurve { id, reason } => format!("e{id}: {reason}"),
        other => other.to_string(),
    }
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: &Extrude,
    earlier: &[FeatureResult],
) -> Result<(Body, Origins), String> {
    let id = feature.sketch;
    let Some(source) = doc.feature(id) else {
        return Err(format!(
            "sketch: there is no feature {id} (was the sketch deleted?)"
        ));
    };
    let name = &source.name;
    let FeatureKind::Sketch(sketch) = &source.kind else {
        return Err(format!(
            "sketch: `{name}` is a {}, not a sketch",
            source.kind.type_name()
        ));
    };
    let Some(outcome) = earlier.iter().find(|r| r.id == id) else {
        return Err(format!(
            "sketch: `{name}` comes after this feature in the history; move it before (feature.move)"
        ));
    };
    match &outcome.status {
        Status::Ok => {}
        Status::Suppressed => return Err(format!("sketch: `{name}` is suppressed")),
        Status::RolledBack => return Err(format!("sketch: `{name}` is rolled back")),
        Status::Failed(_) => {
            return Err(format!(
                "sketch: `{name}` failed; fix it first (sketch.inspect)"
            ));
        }
    }
    let Some(solved) = &outcome.sketch else {
        return Err(format!("sketch: `{name}` has no solution"));
    };

    let point = |entity| {
        solved
            .point(entity)
            .map(|(x, y)| [x, y])
            .ok_or_else(|| format!("sketch `{name}`: {entity} has no solved position"))
    };
    let mut curves = Vec::new();
    for entity in sketch.entities.iter().filter(|e| !e.construction) {
        let curve = match entity.geometry {
            Geometry::Point { .. } => continue,
            Geometry::Line { start, end } => Curve2::Line {
                start: point(start)?,
                end: point(end)?,
            },
            Geometry::Arc { center, start, end } => Curve2::Arc {
                center: point(center)?,
                start: point(start)?,
                end: point(end)?,
            },
            Geometry::Circle { center, .. } => Curve2::Circle {
                center: point(center)?,
                radius: solved.radius(entity.id).ok_or_else(|| {
                    format!("sketch `{name}`: {} has no solved radius", entity.id)
                })?,
            },
        };
        curves.push(SketchCurve {
            id: entity.id.0,
            curve,
        });
    }
    if curves.is_empty() {
        return Err(format!(
            "sketch `{name}` has nothing to extrude: draw a closed profile of lines, arcs or circles"
        ));
    }

    let distance = length(doc, variables, "distance", &feature.distance)?;
    if !distance.is_finite() || distance == 0.0 {
        return Err(format!(
            "distance must be a length other than zero, but `{}` is {distance}",
            feature.distance.source()
        ));
    }
    let (from, to) = if feature.symmetric {
        (-distance.abs() / 2.0, distance.abs() / 2.0)
    } else if distance > 0.0 {
        (0.0, distance)
    } else {
        (distance, 0.0)
    };
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let plane = Frame::new(solved.origin, solved.normal, solved.x_axis)
        .ok_or_else(|| format!("sketch `{name}`: its plane is not finite"))?;
    let profiles = detect_profiles(&curves, &tolerance)
        .map_err(|e| format!("sketch `{name}`: {}", profile_error(&e)))?;
    let mut result = extrude(&profiles, &plane, from, to, &tolerance).map_err(|e| e.to_string())?;
    // For the names, the extrusion starts in the sketch plane and ends at
    // the far side, whichever way it goes. The kernel calls the lower
    // offset the start, so an extrusion against the normal has the two
    // swapped. (A symmetric one starts on the side the normal points
    // away from.)
    if !feature.symmetric && distance < 0.0 {
        let origins = &mut result.origins;
        for origin in origins
            .faces
            .iter_mut()
            .chain(&mut origins.edges)
            .chain(&mut origins.vertices)
        {
            origin.role = match origin.role {
                Role::StartCap => Role::EndCap,
                Role::EndCap => Role::StartCap,
                Role::StartEdge => Role::EndEdge,
                Role::EndEdge => Role::StartEdge,
                Role::StartVertex => Role::EndVertex,
                Role::EndVertex => Role::StartVertex,
                Role::Side => Role::Side,
                Role::LateralEdge => Role::LateralEdge,
            };
        }
    }
    Ok((result.body, result.origins))
}
