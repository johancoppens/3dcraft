//! The sweep along a path: a solid one closed profile sweeps out as it is
//! carried along the curves of another sketch. (`sweep_cmds.rs` holds the
//! revolution, the sweep about an axis.)

use crate::commands::{Run, add_sweep, combine_param, doc_error, fail, feature_ref, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Document, FeatureId, FeatureKind, Sweep};
use craft3d_regen::SweepReport;
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.sweep", "Sweep", r#"{"sketch": id or name, "path": sketch or datum helix, id or name, "cut": id or name, "join": id or name, "intersect": id or name, "name": string = automatic}"#,
        "Adds the solid the closed profile of a sketch sweeps out as it is carried along a path, staying square to it: a pipe along a bent centre line, a handle, a rail, a gasket round a closed path. sketch holds the one closed profile (lines, arcs, splines, or a circle; loops inside it are holes and are swept with it). path is another sketch, on a plane that crosses the profile's plane: one chain of lines, arcs and splines that meet end to end, open or closed, or one circle (construction geometry is not part of it). The path has to pass through the profile's plane at a right angle: an open path at one of its ends, where the sweep starts; a closed path anywhere. The profile need not be centred on the path nor contain it: it keeps its place beside the path all the way. Where two curves of the path meet they are tangent, or, two straight lines only, at a corner, which is mitred (both sides end on the plane that bisects it). Along lines and arcs every face is exact (planes, cylinders, cones, tori); along a spline the faces are B-spline surfaces within half the document's tolerance of the true sweep, and the result says under path how far off they were found to be (deviation) and through how many stations the spline was followed. Refused, with the place: a bend tighter than the profile is wide on its inside, a mitre too sharp for the line it is on, a corner at an arc or a spline, a path that branches or is in several parts, and a sweep that passes through itself. A face is named after the curve of the profile and the curve of the path it comes from (F9:side(e3,e12)); a line of the profile that is parallel to the path's plane sweeps one flat face along the whole path (F9:side(e3)); the ends of an open path are F9:startCap and F9:endCap. path can also be a datum helix (datum.helix): the profile is then carried by the helix's screw motion, turning about its axis and advancing one pitch per turn as a rigid body, so a profile in a plane through the axis stays in such a plane (a thread) and one square to the helix stays square to it (a spring's wire); the profile lies on a plane through the start of the helix, to one side of the axis. Its faces are one per curve of the profile per half turn, B-spline surfaces within half the tolerance (a line of the profile parallel to the axis sweeps a cylinder), named F9:side(e3), then F9:piece(1)/F9:side(e3) and so on in the order they are made, the ends F9:startCap and F9:endCap. Refused, with the reason: a profile that reaches the axis or round it, a pitch that is not longer than the profile is along the axis (one turn would run into the next), a profile whose plane the helix runs along somewhere. cut, join and intersect combine the solid with a body at once, as for feature.extrude.", doc: true, feature_sweep),
];

/// What a sweep says of how its path was followed, for a command's
/// result.
pub(crate) fn report_json(report: &SweepReport) -> Value {
    json!({
        "closed": report.closed,
        "length": report.length,
        "deviation": report.deviation,
        "stations": report.stations,
    })
}

fn feature_sweep(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.sweep";
    let p = Params::new(
        ID,
        params,
        &["sketch", "path", "cut", "join", "intersect", "name"],
    )?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, ID, "sketch")?;
    doc.sketch(sketch).map_err(doc_error(ID))?;
    let path = feature_ref(doc, &p, ID, "path")?;
    path_ref(doc, path, ID)?;
    if sketch == path {
        return fail(format!(
            "{ID}: `sketch` and `path` are the same sketch; draw the path in a sketch of its own, on a plane that crosses the profile's"
        ));
    }
    let combine = combine_param(doc, &p, ID)?;
    let kind = FeatureKind::Sweep(Sweep { sketch, path });
    add_sweep(session, ID, p.str("name")?, kind, combine)
}

/// The path of a sweep has to be a sketch or a datum helix.
pub(crate) fn path_ref(doc: &Document, path: FeatureId, command: &str) -> Result<(), CommandError> {
    match doc.feature(path).map(|f| &f.kind) {
        Some(FeatureKind::DatumHelix(_)) => Ok(()),
        _ => doc.sketch(path).map(|_| ()).map_err(doc_error(command)),
    }
}
