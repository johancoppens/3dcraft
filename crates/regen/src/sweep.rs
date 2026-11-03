//! Sweeps in regeneration: the one closed profile of a solved sketch is
//! carried along the curves of another, or along a datum helix; the body the kernel makes is
//! searched for places where it passes through itself before it is
//! handed on.

use crate::datum::datum_helix;
use crate::extrude::{name_of, profile_error, swept};
use crate::selfcheck::{Crossing, crossing};
use crate::sketch_pattern::CurveIds;
use crate::{FeatureResult, Names, SweepReport};
use craft3d_doc::{Document, FeatureId, FeatureKind, NameRole, PersistentName, Sweep, SweepScale};
use craft3d_kernel::KernelError;
use craft3d_kernel::helix::sweep_helix;
use craft3d_kernel::sweep::sweep;
use craft3d_kernel::sweep2::{Scaling, sweep_two_rails};
use craft3d_topo::Body;
use std::sync::Arc;

pub(crate) fn build(
    doc: &Document,
    id: FeatureId,
    feature: &Sweep,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, SweepReport), String> {
    if feature.sketch == feature.path {
        let name = doc
            .feature(feature.path)
            .map_or_else(|| feature.path.to_string(), |f| f.name.clone());
        return Err(format!(
            "path: `{name}` is the profile and the path; draw the path in a sketch of its own, on a plane the profile's plane crosses"
        ));
    }
    if let Some(rail) = feature.rail {
        return two_rails(doc, id, feature, rail, earlier);
    }
    let made = swept(doc, earlier, feature.sketch, "sweep")?;
    let name = made.name;
    let [profile] = made.profiles.as_slice() else {
        return Err(format!(
            "sketch: sketch `{name}` has {} closed profiles; a sweep carries one (it may have holes)",
            made.profiles.len()
        ));
    };
    if crate::offset_profile::made(doc, earlier, feature.path).is_some()
        || doc
            .feature(feature.path)
            .is_some_and(|f| matches!(f.kind, FeatureKind::OffsetProfile(_)))
    {
        return Err(
            "path: an offset as the path of a sweep is not supported yet; sweep along the sketch"
                .to_string(),
        );
    }
    // The table that names the kernel's numbers of the curves swept.
    let mut ids: Option<CurveIds> = None;
    let helical = doc
        .feature(feature.path)
        .is_some_and(|f| matches!(f.kind, FeatureKind::DatumHelix(_)));
    let swept = if helical {
        let helix = datum_helix(doc, earlier, feature.path, "path")?;
        let along = doc
            .feature(feature.path)
            .map_or_else(|| feature.path.to_string(), |f| f.name.clone());
        sweep_helix(profile, &made.plane, &helix, None, &made.tolerance).map_err(|error| {
            match &error {
                KernelError::Sweep { reason, .. } => format!(
                    "path: helix `{along}`: {}",
                    made.solved.curve_ids.rename_in(reason)
                ),
                KernelError::DegenerateCurve { .. }
                | KernelError::ShortSplinePiece { .. }
                | KernelError::DuplicateCurveId { .. }
                | KernelError::TooManyCurves { .. } => {
                    format!(
                        "path: helix `{along}`: {}",
                        profile_error(&error, &made.solved.curve_ids)
                    )
                }
                other => format!("path: helix `{along}`: {other}"),
            }
        })?
    } else {
        // What `drawn` says of a sketch, as what it says of the path.
        let path = crate::extrude::drawn_plain(doc, earlier, feature.path, "a sweep's path")
            .map_err(|reason| {
                let reason = reason.strip_prefix("sketch: ").unwrap_or(&reason);
                format!("path: {reason}")
            })?;
        let along = path.name;
        // The kernel takes the numbers of both sketches' curves; those of
        // nested patterns are sketch-local, so the path's are moved past
        // the profile's, and both are named from one table.
        let (joint, by) = made.solved.curve_ids.joined(&path.solved.curve_ids);
        let mut curves = path.curves.clone();
        for curve in &mut curves {
            curve.id = CurveIds::moved(curve.id, by);
        }
        ids = Some(joint);
        let joint = ids.as_ref().unwrap_or(&made.solved.curve_ids);
        sweep(profile, &made.plane, &curves, &path.plane, &made.tolerance).map_err(|error| {
            match &error {
                // The kernel names the curves of the path as the sketch does.
                KernelError::Sweep { reason, .. } => {
                    format!("path: sketch `{along}`: {}", joint.rename_in(reason))
                }
                KernelError::DegenerateCurve { .. }
                | KernelError::ShortSplinePiece { .. }
                | KernelError::DuplicateCurveId { .. }
                | KernelError::TooManyCurves { .. } => {
                    format!("path: sketch `{along}`: {}", profile_error(&error, joint))
                }
                other => format!("path: {other}"),
            }
        })?
    };
    let mut faces: Vec<PersistentName> = swept.faces.iter().map(|o| name_of(id, o)).collect();
    // A side is named after a curve of the profile and one of the path:
    // both into their names' words, from the table of the two.
    if let Some(joint) = ids.as_ref().filter(|j| !j.is_empty()) {
        for face in &mut faces {
            let mut link = face;
            while link.role == NameRole::Piece {
                let Some(next) = link.of.as_mut().map(Arc::make_mut) else {
                    break;
                };
                link = next;
            }
            link.sources = joint.expand(&link.sources);
        }
    }
    crate::collapse_runs(
        doc,
        earlier,
        feature.sketch,
        &mut faces,
        &[NameRole::StartCap, NameRole::EndCap],
    );
    crate::offset_profile::rename(
        doc,
        earlier,
        feature.sketch,
        &mut faces,
        &[NameRole::StartCap, NameRole::EndCap],
    )?;

    // A path that comes back near itself carries the profile through
    // what it swept before: a body that is valid in how it hangs together
    // and passes through itself. It is never handed on.
    let advice =
        "keep the path further from itself than the profile is wide, or make the profile narrower";
    match crossing(&swept.body, &faces, made.tolerance.linear) {
        Crossing::None => {}
        Crossing::Spent => return Err(Crossing::spent("path", "sweep")),
        Crossing::Unknown(error) => {
            return Err(format!(
                "path: the sweep cannot be meshed to see whether it passes through itself ({error}); {advice}"
            ));
        }
        touching @ Crossing::Touching(..) => {
            touching.refused("path", "sweep", advice)?;
        }
        Crossing::Found(pair) => {
            let between = pair
                .map(|[a, b]| format!(": {a} and {b} cross"))
                .unwrap_or_default();
            return Err(format!(
                "path: the sweep passes through itself{between}; {advice}"
            ));
        }
    }

    let names = Names::by_faces(id, &swept.body, faces, made.tolerance.linear)?;
    let report = SweepReport {
        closed: swept.closed,
        length: swept.length,
        deviation: swept.deviation,
        stations: swept.stations,
    };
    Ok((swept.body, names, report))
}

/// A sweep along two rails: the profile of `feature.sketch` carried so
/// that its two points where the rails start follow the curves of
/// `feature.path` and of `rail`.
fn two_rails(
    doc: &Document,
    id: FeatureId,
    feature: &Sweep,
    rail: FeatureId,
    earlier: &[FeatureResult],
) -> Result<(Body, Names, SweepReport), String> {
    let feature_name = |f: FeatureId| {
        doc.feature(f)
            .map_or_else(|| f.to_string(), |x| x.name.clone())
    };
    for (key, other) in [("path", feature.path), ("rail", rail)] {
        if other == feature.sketch {
            return Err(format!(
                "{key}: `{}` is the profile and a rail; draw each rail in a sketch of its own",
                feature_name(other)
            ));
        }
    }
    if rail == feature.path {
        return Err(format!(
            "rail: `{}` is both rails; draw the second rail in a sketch of its own",
            feature_name(rail)
        ));
    }
    for (key, other) in [("path", feature.path), ("rail", rail)] {
        if doc.feature(other).is_some_and(|f| {
            matches!(
                f.kind,
                FeatureKind::DatumHelix(_) | FeatureKind::OffsetProfile(_)
            )
        }) {
            return Err(format!(
                "{key}: `{}` is not a sketch; a sweep along two rails takes two sketches, each one open chain of lines, arcs and splines",
                feature_name(other)
            ));
        }
    }
    let made = swept(doc, earlier, feature.sketch, "sweep")?;
    let name = made.name;
    let [profile] = made.profiles.as_slice() else {
        return Err(format!(
            "sketch: sketch `{name}` has {} closed profiles; a sweep carries one (it may have holes)",
            made.profiles.len()
        ));
    };
    let drawn = |key: &'static str, of: FeatureId| {
        crate::extrude::drawn_plain(doc, earlier, of, "a rail of a sweep").map_err(|reason| {
            let reason = reason.strip_prefix("sketch: ").unwrap_or(&reason);
            format!("{key}: {reason}")
        })
    };
    let first = drawn("path", feature.path)?;
    let second = drawn("rail", rail)?;
    // The kernel takes the numbers of three sketches' curves; those of
    // nested patterns are sketch-local, so the rails' are moved past the
    // profile's, and all are named from one table.
    let (joint, by_first) = made.solved.curve_ids.joined(&first.solved.curve_ids);
    let (joint, by_second) = joint.joined(&second.solved.curve_ids);
    let moved = |curves: &[craft3d_kernel::profile::SketchCurve], by: u64| {
        let mut curves = curves.to_vec();
        for curve in &mut curves {
            curve.id = CurveIds::moved(curve.id, by);
        }
        curves
    };
    let curves_first = moved(&first.curves, by_first);
    let curves_second = moved(&second.curves, by_second);
    let scaling = match feature.scale {
        SweepScale::Uniform => Scaling::Uniform,
        SweepScale::Width => Scaling::Width,
    };
    let swept = sweep_two_rails(
        profile,
        &made.plane,
        &curves_first,
        &first.plane,
        &curves_second,
        &second.plane,
        scaling,
        &made.tolerance,
    )
    .map_err(|error| match &error {
        KernelError::Sweep { reason, .. } => format!("rail: {}", joint.rename_in(reason)),
        KernelError::DegenerateCurve { .. }
        | KernelError::ShortSplinePiece { .. }
        | KernelError::DuplicateCurveId { .. }
        | KernelError::TooManyCurves { .. } => format!("rail: {}", profile_error(&error, &joint)),
        other => format!("rail: {other}"),
    })?;
    let mut faces: Vec<PersistentName> = swept.faces.iter().map(|o| name_of(id, o)).collect();
    if !joint.is_empty() {
        for face in &mut faces {
            let mut link = face;
            while link.role == NameRole::Piece {
                let Some(next) = link.of.as_mut().map(Arc::make_mut) else {
                    break;
                };
                link = next;
            }
            link.sources = joint.expand(&link.sources);
        }
    }
    crate::collapse_runs(
        doc,
        earlier,
        feature.sketch,
        &mut faces,
        &[NameRole::StartCap, NameRole::EndCap],
    );
    crate::offset_profile::rename(
        doc,
        earlier,
        feature.sketch,
        &mut faces,
        &[NameRole::StartCap, NameRole::EndCap],
    )?;
    let advice = "keep the rails further from themselves and each other than the section is wide, or make the section smaller";
    match crossing(&swept.body, &faces, made.tolerance.linear) {
        Crossing::None => {}
        Crossing::Spent => return Err(Crossing::spent("rail", "sweep")),
        Crossing::Unknown(error) => {
            return Err(format!(
                "rail: the sweep cannot be meshed to see whether it passes through itself ({error}); {advice}"
            ));
        }
        touching @ Crossing::Touching(..) => {
            touching.refused("rail", "sweep", advice)?;
        }
        Crossing::Found(pair) => {
            let between = pair
                .map(|[a, b]| format!(": {a} and {b} cross"))
                .unwrap_or_default();
            return Err(format!(
                "rail: the sweep passes through itself{between}; {advice}"
            ));
        }
    }
    let names = Names::by_faces(id, &swept.body, faces, made.tolerance.linear)?;
    let report = SweepReport {
        closed: false,
        length: swept.length,
        deviation: swept.deviation,
        stations: swept.stations,
    };
    Ok((swept.body, names, report))
}
