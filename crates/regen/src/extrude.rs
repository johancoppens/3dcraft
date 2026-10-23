//! Extrusions in regeneration: the solved sketch becomes plain curves, the
//! kernel finds the closed profiles and sweeps them.

use crate::cause::FIX_IT_FIRST;
use crate::query::faces_named;
use crate::sketch::same_plane;
use crate::sketch_pattern::{CurveIds, curve_name, instance_curves};
use crate::{FeatureResult, Status, length};
use craft3d_doc::{
    Document, Extrude, ExtrudeEnd, FeatureId, FeatureKind, Geometry, NameRole, PersistentName,
};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::extrude::{Origin, Origins, Role, extrude};
use craft3d_kernel::extrude_to::{Target, extrude_up_to, profile_box};
use craft3d_kernel::profile::{Curve2, SketchCurve, Spline2, detect_profiles};
use craft3d_math::{Frame, Tolerance, Vec3};
use craft3d_topo::{Body, FaceId};

/// The persistent name of an entity an extrusion made: the feature, the
/// part the entity plays and the sketch entities it came from, such as
/// `F2:side(e7)` or `F2:lateralEdge(e7,e8)`; with a number in front when
/// those give more than one (`F2:piece(1)/F2:side(e7)`). It does not depend on the order
/// or number of entities in the body (docs/design/names.md).
pub fn name_of(feature: FeatureId, origin: &Origin) -> PersistentName {
    // The second and later entities the same sources give in a role (the
    // pieces of a cut spline, the halves of a full turn) are numbered:
    // `F2:piece(1)/F2:side(e7)`.
    PersistentName::piece(origin.piece, sweep_name(feature, origin))
}

fn sweep_name(feature: FeatureId, origin: &Origin) -> PersistentName {
    PersistentName::new(
        feature,
        match origin.role {
            Role::StartCap => NameRole::StartCap,
            Role::EndCap => NameRole::EndCap,
            Role::Side => NameRole::Side,
            Role::StartEdge => NameRole::StartEdge,
            Role::EndEdge => NameRole::EndEdge,
            Role::LateralEdge => NameRole::LateralEdge,
            Role::StartVertex => NameRole::StartVertex,
            Role::EndVertex => NameRole::EndVertex,
        },
        origin.sources.clone(),
    )
}

/// Why a sketch has no usable profile, with its curves called what the
/// sketch calls them (`e7`).
pub(crate) fn profile_error(error: &KernelError, ids: &CurveIds) -> String {
    let curve_name = |id: u64| curve_name(id, ids);
    let list = |curves: &[u64]| {
        curves
            .iter()
            .map(|id| curve_name(*id))
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
        KernelError::CrossingCurves { curves: [a, b] } if a == b => format!(
            "{} crosses or touches itself, or has a cusp; the boundary of a profile is a simple curve",
            curve_name(*a)
        ),
        KernelError::CrossingCurves { curves } => format!(
            "{} cross or touch each other; the loops of a profile must stay apart",
            list(curves)
        ),
        KernelError::ZeroAreaLoop { curves } => {
            format!("the loop of {} encloses no area", list(curves))
        }
        KernelError::DegenerateCurve { id, reason } => format!("{}: {reason}", curve_name(*id)),
        KernelError::CrowdedCurves { curves, limit } => format!(
            "the curves lie too close together for profile detection: more than {limit} pairs of curves, curve ends or loops are near one another (about {}); remove curves drawn on top of each other or loops stacked inside each other",
            list(curves)
        ),
        KernelError::TooManySplinePoints {
            count,
            limit,
            curves,
        } => format!(
            "the splines have {count} control points in all, more than profile detection compares ({limit}); the largest are {}: use fewer control points",
            list(curves)
        ),
        other => other.to_string(),
    }
}

/// What a sketch gives a sweep: its closed profiles and where it lies.
pub(crate) struct Swept<'a> {
    /// The sketch's name, for messages.
    pub name: &'a str,
    pub sketch: &'a craft3d_doc::Sketch,
    pub solved: &'a crate::SketchResult,
    pub profiles: Vec<craft3d_kernel::profile::Profile>,
    pub plane: Frame,
    pub tolerance: Tolerance,
}

/// What a sketch gives a feature that is made from its curves: the
/// curves that are not construction, and where it lies.
pub(crate) struct Drawn<'a> {
    /// The sketch's name, for messages.
    pub name: &'a str,
    pub sketch: &'a craft3d_doc::Sketch,
    pub solved: &'a crate::SketchResult,
    pub curves: Vec<SketchCurve>,
    pub plane: Frame,
    pub tolerance: Tolerance,
}

/// The profiles of sketch `id`, for a feature that sweeps them: the
/// sketch has to exist, come earlier, have solved, and hold at least one
/// closed profile. `verb` is what the feature does with them ("extrude").
pub(crate) fn swept<'a>(
    doc: &'a Document,
    earlier: &'a [FeatureResult],
    id: FeatureId,
    verb: &str,
) -> Result<Swept<'a>, String> {
    let Drawn {
        name,
        sketch,
        solved,
        curves,
        plane,
        tolerance,
    } = drawn(doc, earlier, id)?;
    if curves.is_empty() {
        return Err(format!(
            "sketch `{name}` has nothing to {verb}: draw a closed profile of lines, arcs, splines or circles"
        ));
    }
    let profiles = detect_profiles(&curves, &tolerance)
        .map_err(|e| format!("sketch `{name}`: {}", profile_error(&e, &solved.curve_ids)))?;
    Ok(Swept {
        name,
        sketch,
        solved,
        profiles,
        plane,
        tolerance,
    })
}

/// The curves of sketch `id` that are not construction, as the kernel
/// takes them, for a feature that is made from them: the sketch has to
/// exist, come earlier and have solved.
pub(crate) fn drawn<'a>(
    doc: &'a Document,
    earlier: &'a [FeatureResult],
    id: FeatureId,
) -> Result<Drawn<'a>, String> {
    let Some(source) = doc.feature(id) else {
        return Err(format!(
            "sketch: there is no feature {id} (was the sketch deleted?)"
        ));
    };
    let name = &source.name;
    if let FeatureKind::OffsetProfile(offset) = &source.kind {
        return offset_drawn(doc, earlier, id, name, offset.sketch);
    }
    let FeatureKind::Sketch(sketch) = &source.kind else {
        return Err(format!(
            "sketch: `{name}` is a {}, not a sketch",
            source.kind.type_name()
        ));
    };
    let Some(outcome) = earlier.iter().find(|r| r.id == id) else {
        return Err(format!(
            "sketch: `{name}` {}",
            crate::comes_after(doc, earlier, id)
        ));
    };
    match &outcome.status {
        Status::Ok => {}
        Status::Suppressed => return Err(format!("sketch: `{name}` is suppressed")),
        Status::Inactive => return Err(format!("sketch: `{name}` is inactive")),
        Status::RolledBack => return Err(format!("sketch: `{name}` is rolled back")),
        Status::Failed(_) => {
            return Err(format!("sketch: `{name}` {FIX_IT_FIRST} (sketch.inspect)"));
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
        let curve = match &entity.geometry {
            Geometry::Point { .. } | Geometry::Image { .. } => continue,
            &Geometry::Line { start, end } => Curve2::Line {
                start: point(start)?,
                end: point(end)?,
            },
            &Geometry::Arc { center, start, end } => Curve2::Arc {
                center: point(center)?,
                start: point(start)?,
                end: point(end)?,
            },
            Geometry::Spline { degree, control } => {
                let knots = craft3d_doc::clamped_uniform_knots(*degree, control.len());
                let degree = usize::try_from(*degree).unwrap_or(usize::MAX);
                let control = control
                    .iter()
                    .map(|id| point(*id))
                    .collect::<Result<Vec<_>, _>>()?;
                // Knots that could not be made (a degree or a count the
                // document does not allow) are refused by the kernel.
                Curve2::Spline(Spline2 {
                    degree,
                    knots,
                    control,
                })
            }
            &Geometry::Circle { center, .. } => Curve2::Circle {
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
    // The instances of the sketch's patterns, after the curves drawn.
    let instances = instance_curves(sketch, solved, false)
        .map_err(|reason| format!("sketch `{name}`: {reason}"))?;
    curves.extend(instances.into_iter().map(|(curve, _)| curve));
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let plane = Frame::new(solved.origin, solved.normal, solved.x_axis)
        .ok_or_else(|| format!("sketch `{name}`: its plane is not finite"))?;
    Ok(Drawn {
        name,
        sketch,
        solved,
        curves,
        plane,
        tolerance,
    })
}

/// What an offset of the profiles of a sketch gives a feature made from
/// its curves: the offset outline (the kernel's curves are numbered from 0;
/// `offset_profile::rename` names what is made of them), and the plane
/// and solution of its sketch.
fn offset_drawn<'a>(
    doc: &'a Document,
    earlier: &'a [FeatureResult],
    id: FeatureId,
    name: &'a str,
    sketch_id: FeatureId,
) -> Result<Drawn<'a>, String> {
    let Some(outcome) = earlier.iter().find(|r| r.id == id) else {
        return Err(format!(
            "sketch: `{name}` {}",
            crate::comes_after(doc, earlier, id)
        ));
    };
    match &outcome.status {
        Status::Ok => {}
        Status::Suppressed => return Err(format!("sketch: `{name}` is suppressed")),
        Status::Inactive => return Err(format!("sketch: `{name}` is inactive")),
        Status::RolledBack => return Err(format!("sketch: `{name}` is rolled back")),
        Status::Failed(_) => return Err(format!("sketch: `{name}` {FIX_IT_FIRST}")),
    }
    let (Some(made), Ok(sketch)) = (&outcome.offset, doc.sketch(sketch_id)) else {
        return Err(format!("sketch: `{name}` has no outline"));
    };
    let Some(solved) = earlier
        .iter()
        .find(|r| r.id == sketch_id)
        .and_then(|r| r.sketch.as_deref())
    else {
        return Err(format!("sketch: the sketch of `{name}` has no solution"));
    };
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    let plane = Frame::new(solved.origin, solved.normal, solved.x_axis)
        .ok_or_else(|| format!("sketch `{name}`: its plane is not finite"))?;
    Ok(Drawn {
        name,
        sketch,
        solved,
        curves: made.outline.curves.clone(),
        plane,
        tolerance,
    })
}

/// The kernel calls the lower offset (or angle) the start. For the names,
/// a sweep starts in the sketch plane and ends at the far side, whichever
/// way it goes; this swaps the two for a sweep against the positive
/// direction.
pub(crate) fn swap_start_and_end(origins: &mut Origins) {
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

/// The face an extrusion ends on, among the features regenerated so far:
/// the body that has it and its faces of that name (several when a boolean
/// cut the face into pieces). A name that points at nothing is an error
/// that says which reference was lost and what is there instead; it never
/// resolves to another face.
fn target<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    name: &PersistentName,
) -> Result<(&'a Body, Vec<FaceId>), String> {
    if !name.is_face() {
        return Err(format!(
            "upTo: {name} is not a face; an extrusion ends on a face"
        ));
    }
    let Some(source) = doc.feature(name.feature) else {
        return Err(format!(
            "upTo: there is no feature {} for the face {name} (was it deleted?)",
            name.feature
        ));
    };
    let made_by = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == name.feature) else {
        return Err(format!(
            "upTo: `{made_by}`, which makes the face {name}, {}",
            crate::comes_after(doc, earlier, name.feature)
        ));
    };
    if let Some(off) = crate::pass::lost(outcome, name) {
        return Err(format!(
            "upTo: `{made_by}`, which makes the face {name}, is {off}"
        ));
    }
    let (Some(body), Some(names)) = (&outcome.body, &outcome.names) else {
        if let Some(why) = crate::pass::absent(earlier, name.feature) {
            return Err(format!("upTo: `{made_by}`, which makes the face {name}, has no body: {why}"));
        }
        return Err(match &outcome.status {
            Status::Ok => format!(
                "upTo: `{made_by}` is a {}, which has no named faces",
                source.kind.type_name()
            ),
            status @ (Status::Suppressed | Status::Inactive) => {
                format!(
                    "upTo: `{made_by}`, which makes the face {name}, is {}",
                    crate::pass::word(status)
                )
            }
            Status::RolledBack => {
                format!("upTo: `{made_by}`, which makes the face {name}, is rolled back")
            }
            Status::Failed(_) => {
                format!("upTo: `{made_by}`, which makes the face {name}, {FIX_IT_FIRST}")
            }
        });
    };
    if let Some(why) = crate::query::ambiguous_cap(names, name) {
        return Err(format!("upTo: {why}"));
    }
    let mut faces = faces_named(names, name);
    if faces.is_empty() {
        // The caps of one role of one sweep lie on one surface, whatever
        // their outlines: a cap whose outline has changed since it was
        // named still says where to end (as it gives a sketch its plane).
        faces = (0..names.faces.len())
            .filter(|&face| names.names_of(face).any(|own| same_plane(own, name)))
            .filter_map(|face| u32::try_from(face).ok().map(FaceId))
            .collect();
    }
    if faces.is_empty() {
        let there: Vec<String> = names
            .faces
            .iter()
            .take(12)
            .map(ToString::to_string)
            .collect();
        return Err(format!(
            "upTo: `{made_by}` no longer has a face {name}; its faces are: {}{}",
            there.join(", "),
            if names.faces.len() > there.len() {
                ", ..."
            } else {
                ""
            }
        ));
    }
    Ok((body, faces))
}

/// Where an extrusion through all starts and ends, as offsets along the
/// sketch's normal.
///
/// "All" is every body the features before this one have made and that is
/// still a body of the document at this point of the history (not taken
/// by a boolean or a transform before it), as far as it lies in the way:
/// its box, seen along the normal, overlaps the box of the profiles. The
/// extrusion runs from the sketch plane to past the farthest such box, by
/// a margin (a hundredth of the way, at least a thousand tolerances), so
/// that it never ends flush with the far side of what it goes through.
/// `symmetric` goes through on both sides: from before the nearest box to
/// past the farthest.
fn through_all(
    earlier: &[FeatureResult],
    profiles: &[craft3d_kernel::profile::Profile],
    plane: &Frame,
    tolerance: &Tolerance,
    reversed: bool,
    symmetric: bool,
) -> Result<(f64, f64), String> {
    let Some((lo, hi)) = profile_box(profiles) else {
        return Err("the profile is not finite".to_string());
    };
    let tol = tolerance.linear;
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    let mut bodies = 0_usize;
    for body in earlier
        .iter()
        .filter(|r| r.is_built())
        .filter_map(FeatureResult::own_body)
    {
        bodies += 1;
        // A body whose box cannot be told (an unbounded face) is in no
        // one's way; regeneration has checked every body, so this does
        // not happen.
        let Some((a, b)) = body.bbox().ok().and_then(|b| b.corners()) else {
            continue;
        };
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for corner in 0..8_u8 {
            let pick = |bit: u8, lo: f64, hi: f64| if corner & bit == 0 { lo } else { hi };
            let p = Vec3::new(pick(1, a.x, b.x), pick(2, a.y, b.y), pick(4, a.z, b.z));
            let (x, y, z) = plane.local(p);
            min = [min[0].min(x), min[1].min(y), min[2].min(z)];
            max = [max[0].max(x), max[1].max(y), max[2].max(z)];
        }
        let beside = |axis: usize| {
            let (Some(&p0), Some(&p1), Some(&b0), Some(&b1)) =
                (lo.get(axis), hi.get(axis), min.get(axis), max.get(axis))
            else {
                return true;
            };
            b1 < p0 - tol || b0 > p1 + tol
        };
        if beside(0) || beside(1) {
            continue;
        }
        (low, high) = (low.min(min[2]), high.max(max[2]));
    }
    let nothing = |side: &str| {
        format!(
            "throughAll: there is nothing to go through {side}: none of the {bodies} bodies built before this feature lies in the way of the profile{}",
            if symmetric {
                ""
            } else {
                " (reversed goes the other way)"
            }
        )
    };
    let margin = |length: f64| (0.01 * length.abs()).max(1000.0 * tol);
    let ends = if symmetric {
        if low > high {
            return Err(nothing("on either side of the sketch plane"));
        }
        (low - margin(low), high + margin(high))
    } else if reversed {
        if low >= -tol {
            return Err(nothing("against the sketch's normal"));
        }
        (low - margin(low), 0.0)
    } else {
        if high <= tol {
            return Err(nothing("along the sketch's normal"));
        }
        (0.0, high + margin(high))
    };
    if !(ends.0.is_finite() && ends.1.is_finite()) {
        return Err("throughAll: the bodies in the way are not finite".to_string());
    }
    Ok(ends)
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: &Extrude,
    earlier: &[FeatureResult],
) -> Result<(Body, Origins), String> {
    let Swept {
        profiles,
        plane,
        tolerance,
        ..
    } = swept(doc, earlier, feature.sketch, "extrude")?;

    // The kernel calls the lower offset the start; for the names a sweep
    // starts in the sketch plane (a symmetric one on the side the normal
    // points away from). `against` says the two differ.
    let (from, to, against) = match &feature.end {
        ExtrudeEnd::Distance => {
            let distance = length(doc, variables, "distance", &feature.distance)?;
            if !distance.is_finite() || distance == 0.0 {
                return Err(format!(
                    "distance must be a length other than zero, but `{}` is {distance}",
                    feature.distance.source()
                ));
            }
            let distance = if feature.reversed {
                -distance
            } else {
                distance
            };
            if feature.symmetric {
                (-distance.abs() / 2.0, distance.abs() / 2.0, false)
            } else if distance > 0.0 {
                (0.0, distance, false)
            } else {
                (distance, 0.0, true)
            }
        }
        ExtrudeEnd::ThroughAll => {
            let (from, to) = through_all(
                earlier,
                &profiles,
                &plane,
                &tolerance,
                feature.reversed,
                feature.symmetric,
            )?;
            (from, to, feature.reversed && !feature.symmetric)
        }
        ExtrudeEnd::UpToFace { face } => {
            if feature.symmetric {
                return Err(
                    "symmetric: an extrusion up to a face goes one way; it cannot be symmetric"
                        .to_string(),
                );
            }
            let (body, faces) = target(doc, earlier, face)?;
            let made = extrude_up_to(
                &profiles,
                &plane,
                feature.reversed,
                &Target {
                    body,
                    faces: &faces,
                },
                &tolerance,
            )
            .map_err(|e| format!("upTo: the face {face}: {e}"))?;
            return Ok((made.body, made.origins));
        }
    };
    let mut result = extrude(&profiles, &plane, from, to, &tolerance).map_err(|e| e.to_string())?;
    if against {
        swap_start_and_end(&mut result.origins);
    }
    Ok((result.body, result.origins))
}

/// Fails when an extrusion up to a face passes through itself. An
/// extrusion by a distance cannot: its profiles do not touch or cross
/// (`detect_profiles`), so the prisms over them, all along one direction
/// and as long, do not either. One up to a face is the prism cut by a
/// tool with the target's surface for a face (`extrude_to`): its faces
/// are pieces of the faces of the prism (one group) or of that surface,
/// the end caps (the other), and only where the two can meet is looked
/// at (`selfcheck::crossing_near`).
pub(crate) fn not_through_itself(
    body: &Body,
    origins: &Origins,
    names: &crate::Names,
    linear: f64,
) -> Result<(), String> {
    let roles: Vec<craft3d_tess::FaceRole> = origins
        .faces
        .iter()
        .map(|origin| match origin.role {
            Role::EndCap => craft3d_tess::FaceRole::Known(1),
            _ => craft3d_tess::FaceRole::Known(0),
        })
        .collect();
    if roles.len() != body.faces.len() {
        return Err("upTo: the extrusion lost track of its faces; this is a bug".to_string());
    }
    crate::selfcheck::crossing_near(body, &names.faces, linear, &roles).refused(
        "upTo",
        "extrusion",
        "the cut at the face did not find where the extrusion meets it, and the result is refused rather than handed on",
    )
}

/// A fill: the flat faces the closed profiles of the sketch bound, as a
/// sheet. Named as the start cap of an extrusion of the same profiles
/// (`F5:startCap(e1,e2,e3,e4)`, `F5:startEdge(e2)`,
/// `F5:startVertex(e1,e2)`), so a cap's name keeps when a pattern's count
/// changes and an offset's curves name what they bound.
pub(crate) fn fill(
    doc: &Document,
    feature: &craft3d_doc::Fill,
    earlier: &[FeatureResult],
) -> Result<(Body, Origins), String> {
    let Swept {
        profiles,
        plane,
        tolerance,
        ..
    } = swept(doc, earlier, feature.sketch, "fill")?;
    let made =
        craft3d_kernel::extrude::fill(&profiles, &plane, &tolerance).map_err(|e| e.to_string())?;
    Ok((made.body, made.origins))
}
