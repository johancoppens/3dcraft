//! Threads in regeneration: an ISO metric thread cut into a cylindrical
//! face of the body of an earlier feature, or only marked on it.
//!
//! **Where.** The thread lies on the axis of the cylindrical face `face`
//! and starts where that axis passes through the flat face `start`, which
//! is square to the axis at one end of the cylinder (the end of a shank:
//! nothing of the body lies beyond it). It runs from there along the
//! cylinder, into the material, for `length` at full depth, and lifts
//! out over a quarter turn. The face's diameter has to be the size's
//! nominal one (within a part of the thread's depth: a shank turned a
//! little under size is threaded all the same, to the basic profile of
//! the nominal size).
//!
//! **What.** The groove of the basic profile ([`craft3d_kernel::thread`])
//! is swept along a helix at the minor diameter and subtracted from the
//! body by the boolean. A cosmetic thread changes nothing of the body:
//! the feature carries what the thread is ([`ThreadReport`]), so that
//! `document.inspect` shows it with the face.
//!
//! **Names.** A face of the body that is still there is named as a copy,
//! instance 0, of the face it was (`F9:copy(0)/F2:side(e7)`), as after a
//! boolean. The faces of the groove are `F9:thread(0)` (the root),
//! `F9:thread(1)` (the flank that faces the start) and `F9:thread(2)`
//! (the flank that faces away), one per half turn of the groove, the
//! second and later numbered as pieces (`F9:piece(3)/F9:thread(1)`); the
//! groove's other faces lie outside the material and are not left. Edges
//! and vertices are named by the faces they lie on, with the copy of the
//! name they had as an alias, as after a boolean.

use crate::boolean::combine;
use crate::face_ops::{self, Source};
use crate::{FeatureResult, Kept, Names, ThreadReport, length};
use craft3d_doc::{BooleanOp, Document, Expr, FeatureId, NameRole, PersistentName, Thread};
use craft3d_expr::Evaluation;
use craft3d_geom::Surface;
use craft3d_kernel::extrude::Role;
use craft3d_kernel::thread::{IsoThread, RUN_OUT, ThreadPlace, groove, nominal_diameter};
use craft3d_math::Vec3;
use craft3d_topo::{Body, FaceId};

const NOUN: &str = "a thread";

/// The axis and radius of the cylindrical face `name`.
fn cylinder(
    name: &PersistentName,
    source_id: FeatureId,
    source: &Source<'_>,
) -> Result<(Vec3, Vec3, f64, Vec<FaceId>), String> {
    let faces = face_ops::pick(std::slice::from_ref(name), "face", NOUN, source_id, source)?;
    let mut found: Option<(Vec3, Vec3, f64)> = None;
    for face in &faces {
        let Some(surface) = source.body.face(*face).map(|f| &f.surface) else {
            continue;
        };
        let Surface::Cylinder(c) = surface else {
            return Err(format!(
                "face: {name} is a {} face; a thread is cut on a cylinder",
                surface.kind()
            ));
        };
        let here = (c.frame().origin(), c.frame().z(), c.radius());
        match found {
            None => found = Some(here),
            // The pieces a cut divided the face into lie on one cylinder.
            Some((origin, axis, radius)) => {
                let off = (here.0 - origin).cross(axis).length();
                if off > source.tolerance.linear
                    || here.1.cross(axis).length() > 1e-9
                    || (here.2 - radius).abs() > source.tolerance.linear
                {
                    return Err(format!(
                        "face: the faces named {name} lie on different cylinders"
                    ));
                }
            }
        }
    }
    let (origin, axis, radius) =
        found.ok_or_else(|| format!("face: `{}` has no face {name}", source.made_by))?;
    Ok((origin, axis, radius, faces))
}

/// The points of the faces' vertices.
fn corners(body: &Body, faces: &[FaceId]) -> Vec<Vec3> {
    let mut out = Vec::new();
    for face in faces.iter().filter_map(|f| body.face(*f)) {
        for coedge in face
            .loops
            .iter()
            .filter_map(|l| body.loop_(*l))
            .flat_map(|l| &l.coedges)
        {
            let Some(edge) = body.edges.get(coedge.edge.0 as usize) else {
                continue;
            };
            for vertex in [edge.start, edge.end] {
                if let Some(v) = body.vertex(vertex) {
                    out.push(v.point);
                }
            }
        }
    }
    out
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    thread: &Thread,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>, ThreadReport), String> {
    let source_id = thread.source();
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        Some(&thread.face),
        "face",
        NOUN,
    )?;
    if thread.start.feature != source_id {
        return Err(format!(
            "start: {} is a face of {}, not of `{}`; the thread starts at a face of the body it is cut in",
            thread.start, thread.start.feature, source.made_by
        ));
    }
    let (origin, axis, radius, faces) = cylinder(&thread.face, source_id, &source)?;

    // Where the axis passes through the start face, and which way the
    // cylinder goes from there.
    let ends = face_ops::pick(
        std::slice::from_ref(&thread.start),
        "start",
        NOUN,
        source_id,
        &source,
    )?;
    let plane = ends
        .first()
        .and_then(|f| source.body.face(*f))
        .map(|f| &f.surface);
    let Some(Surface::Plane(plane)) = plane else {
        return Err(format!(
            "start: {} is not a flat face; a thread starts at the flat end of the cylinder",
            thread.start
        ));
    };
    let normal = plane.frame().z();
    let slope = normal.dot(axis);
    if !(normal.cross(axis).length() <= 1e-9 && slope.abs() > 0.5) {
        return Err(format!(
            "start: {} is not square to the axis of {}; a thread starts at the end of the cylinder",
            thread.start, thread.face
        ));
    }
    let at = origin + axis * ((plane.frame().origin() - origin).dot(normal) / slope);
    let tol = source.tolerance.linear;
    let reach: Vec<f64> = corners(source.body, &faces)
        .iter()
        .map(|p| (*p - at).dot(axis))
        .collect();
    let (low, high) = reach
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), r| {
            (lo.min(*r), hi.max(*r))
        });
    let (along, extent) = if low >= -tol && high > tol {
        (axis, high)
    } else if high <= tol && low < -tol {
        (-axis, -low)
    } else {
        return Err(format!(
            "start: {} is not at an end of {}; a thread starts where the cylinder ends",
            thread.start, thread.face
        ));
    };

    let unit = doc.units.length.symbol();
    let Some(nominal_mm) = nominal_diameter(&thread.size) else {
        return Err(format!(
            "size: `{}` is not an ISO metric size; give M and the nominal diameter in millimetres, such as M10",
            thread.size
        ));
    };
    let nominal = length(
        doc,
        variables,
        "size",
        &Expr::new(format!("{nominal_mm} mm")),
    )?;
    let pitch = length(doc, variables, "pitch", &thread.pitch)?;
    let run = length(doc, variables, "length", &thread.length)?;
    let iso = IsoThread::new(nominal, pitch).map_err(|_| {
        format!(
            "pitch: {pitch} {unit} is not a pitch for {}: it has to be above 0 and below the diameter",
            thread.size
        )
    })?;
    if pitch.is_nan() || pitch <= tol {
        return Err(format!("pitch: {pitch} {unit} is not above the tolerance"));
    }
    if !(run.is_finite() && run > tol) {
        return Err(format!("length: {run} {unit} is not a length above 0"));
    }
    let diameter = 2.0 * radius;
    if radius > nominal / 2.0 + iso.overshoot() / 2.0 || radius < nominal / 2.0 - iso.depth() / 2.0
    {
        return Err(format!(
            "face: {} is {diameter} {unit} across; {} is cut on a cylinder of its nominal diameter, {nominal} {unit} (minor diameter {} {unit})",
            thread.face,
            thread.size,
            iso.minor()
        ));
    }
    // The groove is half a pitch wide at most on either side of its
    // centre, and the run-out takes it a quarter of a pitch further.
    let needed = run + pitch * (0.5 + RUN_OUT / std::f64::consts::TAU);
    if needed > extent + tol {
        return Err(format!(
            "length: the thread with its run-out reaches {needed} {unit} along {} and the cylinder is {extent} {unit} long; make the thread shorter",
            thread.face
        ));
    }
    let mut report = ThreadReport {
        size: thread.size.clone(),
        major: nominal,
        minor: iso.minor(),
        pitch,
        length: run,
        left_handed: thread.left_handed,
        cosmetic: thread.cosmetic,
        deviation: 0.0,
    };

    if thread.cosmetic {
        // Nothing of the body changes: every face, edge and vertex is
        // the copy of what it was.
        let count = |n: usize| -> Kept { (0..n).map(|i| Some((0, i))).collect() };
        let contributors: Vec<Vec<(u64, usize)>> =
            (0..source.body.faces.len()).map(|i| vec![(0, i)]).collect();
        let names = Names::copied_faces(feature, &contributors, |_| Some(source.names))?.map(
            |mut names| {
                names.name_entities(
                    feature,
                    source.body,
                    tol,
                    &count(source.body.edges.len()),
                    &count(source.body.vertices.len()),
                    |_| Some(source.names),
                );
                names
            },
        );
        return Ok((source.body.clone(), names, report));
    }

    let across = crate::datum::across(along);
    let place = ThreadPlace {
        start: at,
        along,
        across,
        length: run,
        left_handed: thread.left_handed,
    };
    let (swept, _) = groove(&iso, &place, &source.tolerance)
        .map_err(|error| format!("the groove of the thread cannot be made: {error}"))?;
    report.deviation = swept.deviation;
    // The groove's faces, by what they are of the profile: the curves of
    // `IsoThread::groove` are 1 the root, 2 the flank towards the start,
    // 3 the line outside, 4 the flank away from it.
    let groove_names = Names {
        faces: swept
            .faces
            .iter()
            .map(|origin| {
                let number = match (origin.role, origin.sources.as_slice()) {
                    (Role::Side, [1]) => 0,
                    (Role::Side, [2]) => 1,
                    (Role::Side, [4]) => 2,
                    (Role::Side, _) => 3,
                    (Role::StartCap, _) => 4,
                    _ => 5,
                };
                PersistentName::piece(
                    origin.piece,
                    PersistentName::new(feature, NameRole::Thread, vec![number]),
                )
            })
            .collect(),
        ..Names::default()
    };
    let made = combine(
        source.body,
        &swept.body,
        BooleanOp::Subtract,
        &source.tolerance,
    )
    .map_err(|error| format!("the groove of the thread cannot be cut: {error}"))?;
    let operand = |instance: u64| {
        if instance == 0 {
            Some(source.names)
        } else {
            Some(&groove_names)
        }
    };
    let Some(mut names) = Names::copied_faces(feature, &made.contributors, operand)? else {
        return Ok((made.body, None, report));
    };
    // The faces of the groove are this feature's own: not copies.
    for name in names
        .faces
        .iter_mut()
        .chain(names.aliases.values_mut().flatten())
    {
        if name.role == NameRole::Copy
            && name.sources == [1]
            && let Some(own) = name.of.take()
        {
            *name = *own;
        }
    }
    names.name_entities(
        feature,
        &made.body,
        tol,
        &made.edges,
        &made.vertices,
        operand,
    );
    Ok((made.body, Some(names), report))
}
