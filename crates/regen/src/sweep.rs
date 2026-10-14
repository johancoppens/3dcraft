//! Sweeps in regeneration: the one closed profile of a solved sketch is
//! carried along the curves of another; the body the kernel makes is
//! searched for places where it passes through itself before it is
//! handed on.

use crate::extrude::{drawn, name_of, profile_error, swept};
use crate::selfcheck::{Crossing, crossing};
use crate::{FeatureResult, Names, SweepReport, entity};
use craft3d_doc::{Document, FeatureId, PersistentName, Sweep};
use craft3d_kernel::KernelError;
use craft3d_kernel::sweep::sweep;
use craft3d_topo::Body;

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
    let made = swept(doc, earlier, feature.sketch, "sweep")?;
    let name = made.name;
    let [profile] = made.profiles.as_slice() else {
        return Err(format!(
            "sketch: sketch `{name}` has {} closed profiles; a sweep carries one (it may have holes)",
            made.profiles.len()
        ));
    };
    // What `drawn` says of a sketch, as what it says of the path.
    let path = drawn(doc, earlier, feature.path).map_err(|reason| {
        let reason = reason.strip_prefix("sketch: ").unwrap_or(&reason);
        format!("path: {reason}")
    })?;
    let along = path.name;
    let swept = sweep(
        profile,
        &made.plane,
        &path.curves,
        &path.plane,
        &made.tolerance,
    )
    .map_err(|error| match &error {
        // The kernel names the curves of the path as the sketch does.
        KernelError::Sweep { reason, .. } => format!("path: sketch `{along}`: {reason}"),
        KernelError::DegenerateCurve { .. }
        | KernelError::DuplicateCurveId { .. }
        | KernelError::TooManyCurves { .. } => {
            format!("path: sketch `{along}`: {}", profile_error(&error))
        }
        other => format!("path: {other}"),
    })?;
    let faces: Vec<PersistentName> = swept.faces.iter().map(|o| name_of(id, o)).collect();

    // A path that comes back near itself carries the profile through
    // what it swept before: a body that is valid in how it hangs together
    // and passes through itself. It is never handed on.
    let advice =
        "keep the path further from itself than the profile is wide, or make the profile narrower";
    match crossing(&swept.body, &faces, made.tolerance.linear) {
        Crossing::None => {}
        Crossing::Unknown(error) => {
            return Err(format!(
                "path: the sweep cannot be meshed to see whether it passes through itself ({error}); {advice}"
            ));
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

    let by_faces = entity::by_faces(id, &swept.body, &faces, made.tolerance.linear);
    let names = Names {
        faces,
        edges: by_faces.edges,
        vertices: by_faces.vertices,
        ..Names::default()
    };
    let report = SweepReport {
        closed: swept.closed,
        length: swept.length,
        deviation: swept.deviation,
        stations: swept.stations,
    };
    Ok((swept.body, names, report))
}
