//! The sweep along a path: a solid one closed profile sweeps out as it is
//! carried along the curves of another sketch. (`sweep_cmds.rs` holds the
//! revolution, the sweep about an axis.)

use crate::commands::{Run, add_sweep, combine_param, doc_error, fail, feature_ref, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::{Document, FeatureId, FeatureKind, Sweep, SweepScale};
use craft3d_regen::SweepReport;
use serde_json::{Value, json};

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("feature.sweep", "Sweep", r#"{"sketch": id or name, "path": sketch or datum helix, id or name, "rail": sketch, id or name = none, "scale": "uniform" | "width" = "uniform", "cut": id or name, "join": id or name, "intersect": id or name, "name": string = automatic, "when": expr = always on}"#,
        "Adds the solid the closed profile of a sketch sweeps out as it is carried along a path, staying square to it: a pipe along a bent centre line, a handle, a rail, a gasket round a closed path. sketch holds the one closed profile (lines, arcs, splines, or a circle; loops inside it are holes and are swept with it). path is another sketch, on a plane that crosses the profile's plane: one chain of lines, arcs and splines that meet end to end, open or closed, or one circle (construction geometry is not part of it). The path has to pass through the profile's plane at a right angle (a profile that leans off square by so little that laying it square moves no point of it by more than the document's tolerance is laid square; more is refused with the angle and the movement): an open path at one of its ends, where the sweep starts; a closed path anywhere. The profile need not be centred on the path nor contain it: it keeps its place beside the path all the way. Where two curves of the path meet they are tangent, or, two straight lines only, at a corner, which is mitred (both sides end on the plane that bisects it). Along lines and arcs every face is exact (planes, cylinders, cones, tori); along a spline the faces are B-spline surfaces within half the document's tolerance of the true sweep, and the result says under path how far off they were found to be (deviation) and through how many stations the spline was followed. Refused, with the place: a bend tighter than the profile is wide on its inside, a mitre too sharp for the line it is on, a corner at an arc or a spline, a path that branches or is in several parts, and a sweep that passes through itself. A face is named after the curve of the profile and the curve of the path it comes from (F9:side(e3,e12)); a line of the profile that is parallel to the path's plane sweeps one flat face along the whole path (F9:side(e3)); the ends of an open path are F9:startCap and F9:endCap. path can also be a datum helix (datum.helix): the profile is then carried by the helix's screw motion, turning about its axis and advancing one pitch per turn as a rigid body, so a profile in a plane through the axis stays in such a plane (a thread) and one square to the helix stays square to it (a spring's wire); the profile lies on a plane through the start of the helix, to one side of the axis. Its faces are one per curve of the profile per half turn, B-spline surfaces within half the tolerance (a line of the profile parallel to the axis sweeps a cylinder), named F9:side(e3), then F9:piece(1)/F9:side(e3) and so on in the order they are made, the ends F9:startCap and F9:endCap. Refused, with the reason: a profile that reaches the axis or round it, a pitch that is not longer than the profile is along the axis (one turn would run into the next), a profile whose plane the helix runs along somewhere. With rail, a second sketch of one open chain, the sweep is along two rails: path and rail are each one open chain of lines, arcs and splines (each in a sketch of its own, so they may lie in different planes), and each has to start on the outline of the profile. The profile is placed at every station so that the two points where the rails start on it follow the rails, matched by the same fraction of each rail's length (by arc length along lines and arcs, by the spline's parameter within a spline piece). Its x axis is the chord between the rails, its y axis the up direction made square to the chord; the up direction is the normal of the path's sketch, so for rails drawn in one plane the sections stand square to it, and the profile has to be drawn on the plane through both rails' starts that holds that direction. scale is \"uniform\" (the default: the profile grows and shrinks both ways with the distance between the rails, a circle stays a circle) or \"width\" (only along the chord; the height stays as drawn). Where both rails are parallel lines of equal length the faces are exact (planes, cylinders); otherwise they are B-spline surfaces proven within half the document's tolerance of the true sweep (the bound, not a sample, is the result's deviation). Refused, with the place: a rail that does not start on the profile, rails that meet or cross anywhere, rails that start at one point, a profile off the plane of the chord and the up direction, a chord that turns into the up direction, and a sweep that folds or passes through itself. A face is named after the curve of the profile and the curves of the path and the rail it runs between (F9:side(e3,e12,e7)); the ends are F9:startCap and F9:endCap. cut, join and intersect combine the solid with a body at once, as for feature.extrude.", doc: true, feature_sweep),
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
        &[
            "sketch",
            "path",
            "rail",
            "scale",
            "cut",
            "join",
            "intersect",
            "name",
        ],
    )?;
    let doc = session.document()?.history.current();
    let sketch = feature_ref(doc, &p, ID, "sketch")?;
    doc.profile_source(sketch).map_err(doc_error(ID))?;
    let path = feature_ref(doc, &p, ID, "path")?;
    path_ref(doc, path, ID)?;
    if sketch == path {
        return fail(format!(
            "{ID}: `sketch` and `path` are the same sketch; draw the path in a sketch of its own, on a plane that crosses the profile's"
        ));
    }
    let mut sweep = Sweep {
        sketch,
        path,
        rail: None,
        scale: SweepScale::Uniform,
    };
    edit_rails(doc, &p, &mut sweep)?;
    if p.get("scale").is_some() && sweep.rail.is_none() {
        return fail(format!(
            "{ID}: `scale` is for a sweep along two rails; give `rail` too"
        ));
    }
    let combine = combine_param(doc, &p, ID)?;
    let kind = FeatureKind::Sweep(sweep);
    add_sweep(session, ID, p.str("name")?, kind, combine)
}

/// The path of a sweep has to be a sketch or a datum helix.
pub(crate) fn path_ref(doc: &Document, path: FeatureId, command: &str) -> Result<(), CommandError> {
    match doc.feature(path).map(|f| &f.kind) {
        Some(FeatureKind::DatumHelix(_)) => Ok(()),
        _ => doc.sketch(path).map(|_| ()).map_err(doc_error(command)),
    }
}

/// `rail` (a sketch, or null for none) and `scale` (`"uniform"` or
/// `"width"`) of a sweep, as `feature.sweep` and `feature.edit` take them.
pub(crate) fn edit_rails(
    doc: &Document,
    p: &crate::params::Params,
    sweep: &mut Sweep,
) -> Result<(), CommandError> {
    let command = p.command();
    match p.get("rail") {
        None => {}
        Some(Value::Null) => sweep.rail = None,
        Some(_) => {
            let rail = feature_ref(doc, p, command, "rail")?;
            doc.sketch(rail).map_err(doc_error(command))?;
            if rail == sweep.sketch || rail == sweep.path {
                return fail(format!(
                    "{command}: `rail` is the same sketch as `sketch` or `path`; draw each rail in a sketch of its own"
                ));
            }
            if matches!(
                doc.feature(sweep.path).map(|f| &f.kind),
                Some(FeatureKind::DatumHelix(_))
            ) {
                return fail(format!(
                    "{command}: a sweep along two rails takes two sketches; `path` is a datum helix"
                ));
            }
            sweep.rail = Some(rail);
        }
    }
    match p.get("scale") {
        None => {}
        Some(Value::String(text)) if text == "uniform" => sweep.scale = SweepScale::Uniform,
        Some(Value::String(text)) if text == "width" => sweep.scale = SweepScale::Width,
        Some(_) => {
            return fail(format!(
                "{command}: `scale` must be \"uniform\" or \"width\""
            ));
        }
    }
    Ok(())
}
