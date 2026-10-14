//! Reference geometry in regeneration: where a datum lies.
//!
//! A datum makes no body. Its result is a plane, a line or a point in space, worked out
//! from what it is placed relative to: a base plane, an earlier datum (by
//! the result of that feature), a face, an edge or a vertex of an earlier
//! body (by its persistent name, against the body of the feature that
//! made it). A reference that points at nothing fails the datum with the
//! reason and the reference it lost; it never resolves to something else.

use crate::cause::FIX_IT_FIRST;
use crate::query::{edges_between, faces_named};
use crate::{FeatureResult, Status, angle, length};
use craft3d_doc::{
    AxisBy, AxisRef, DatumAxis, DatumHelix, DatumPlane, DatumPoint, Document, Expr, FeatureId,
    HelixExtent, HelixStart, PersistentName, PlaneBy, PlaneRef, PointRef, WorldAxis, WorldPlane,
};
use craft3d_expr::Evaluation;
use craft3d_geom::{Curve3, Surface};
use craft3d_kernel::helix::Helix;
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::Body;

/// Where a datum lies, in the document's length unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Datum {
    /// A plane with an origin and axes: z is its normal.
    Plane(Frame),
    /// A straight line: a point on it and its unit direction.
    Axis {
        origin: Vec3,
        direction: Vec3,
    },
    Point(Vec3),
    /// A helix: its axis, start, radius, pitch, turns and handedness.
    Helix(Helix),
}

impl Datum {
    /// What the datum is, for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Datum::Plane(_) => "datum plane",
            Datum::Axis { .. } => "datum axis",
            Datum::Point(_) => "datum point",
            Datum::Helix(_) => "datum helix",
        }
    }
}

/// A straight line: a point on it and its unit direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    pub origin: Vec3,
    pub direction: Vec3,
}

/// The frame of a base plane: sketch x, sketch y and the normal.
pub(crate) fn world_frame(plane: WorldPlane) -> Result<Frame, String> {
    let (x, normal) = match plane {
        WorldPlane::Xy => (Vec3::X, Vec3::Z),
        WorldPlane::Yz => (Vec3::Y, Vec3::X),
        WorldPlane::Zx => (Vec3::Z, Vec3::Y),
    };
    Frame::new(Vec3::ZERO, normal, x).ok_or_else(|| "a base plane has no frame".to_string())
}

/// The result of the datum feature `id`, for a feature that is placed by
/// it: the datum has to exist, come earlier and have succeeded. `role` is
/// the parameter that refers to it, `wanted` what it has to be.
pub(crate) fn datum_of(
    doc: &Document,
    earlier: &[FeatureResult],
    id: FeatureId,
    role: &str,
    wanted: &str,
) -> Result<Datum, String> {
    let Some(source) = doc.feature(id) else {
        return Err(format!(
            "{role}: there is no feature {id} (was the {wanted} deleted?)"
        ));
    };
    let name = &source.name;
    if !source.kind.is_datum() {
        return Err(format!(
            "{role}: `{name}` is a {}, not a {wanted}",
            source.kind.type_name()
        ));
    }
    let Some(outcome) = earlier.iter().find(|r| r.id == id) else {
        return Err(format!(
            "{role}: `{name}` {}",
            crate::comes_after(doc, earlier, id)
        ));
    };
    match &outcome.status {
        Status::Ok => {}
        Status::Suppressed => return Err(format!("{role}: `{name}` is suppressed")),
        Status::RolledBack => return Err(format!("{role}: `{name}` is rolled back")),
        Status::Failed(_) => return Err(format!("{role}: `{name}` {FIX_IT_FIRST}")),
    }
    outcome
        .datum
        .ok_or_else(|| format!("{role}: `{name}` has no result"))
}

/// "`Shelf` is a datum plane, not a datum axis".
fn not_a(doc: &Document, id: FeatureId, role: &str, found: &Datum, wanted: &str) -> String {
    let name = doc
        .feature(id)
        .map_or_else(|| id.to_string(), |f| f.name.clone());
    format!("{role}: `{name}` is a {}, not a {wanted}", found.kind())
}

/// The frame of the datum plane `id`.
pub(crate) fn datum_plane(
    doc: &Document,
    earlier: &[FeatureResult],
    id: FeatureId,
    role: &str,
) -> Result<Frame, String> {
    match datum_of(doc, earlier, id, role, "datum plane")? {
        Datum::Plane(frame) => Ok(frame),
        other => Err(not_a(doc, id, role, &other, "datum plane")),
    }
}

/// The line of the datum axis `id`.
pub(crate) fn datum_axis(
    doc: &Document,
    earlier: &[FeatureResult],
    id: FeatureId,
    role: &str,
) -> Result<Line, String> {
    match datum_of(doc, earlier, id, role, "datum axis")? {
        Datum::Axis { origin, direction } => Ok(Line { origin, direction }),
        other => Err(not_a(doc, id, role, &other, "datum axis")),
    }
}

pub(crate) fn plane_ref(
    doc: &Document,
    earlier: &[FeatureResult],
    plane: &PlaneRef,
    role: &str,
) -> Result<Frame, String> {
    match plane {
        PlaneRef::World { plane } => world_frame(*plane),
        PlaneRef::Datum { feature } => datum_plane(doc, earlier, *feature, role),
        // As a sketch finds the face it lies on: the same frame, with the
        // normal pointing out of the body.
        PlaneRef::Face { face } => crate::sketch::resolve_face(doc, earlier, face).map_err(|e| {
            let reason = e.strip_prefix("plane: ").unwrap_or(&e);
            format!("{role}: {reason}")
        }),
    }
}

/// The body of the feature a name is of, with what its entities are
/// called.
fn named_body<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    name: &PersistentName,
    role: &str,
) -> Result<(&'a Body, &'a crate::Names, String), String> {
    named_body_of(doc, earlier, name, None, role)
}

/// As [`named_body`], for a reference made of two names of one feature
/// (the two faces an edge lies between): a reason names both, since both
/// are lost with the feature.
pub(crate) fn named_body_of<'a>(
    doc: &Document,
    earlier: &'a [FeatureResult],
    first: &PersistentName,
    second: Option<&PersistentName>,
    role: &str,
) -> Result<(&'a Body, &'a crate::Names, String), String> {
    let name = &match second {
        Some(second) if second != first => format!("{first} and {second}"),
        _ => first.to_string(),
    };
    let Some(source) = doc.feature(first.feature) else {
        return Err(format!(
            "{role}: there is no feature {} for {name} (was it deleted?)",
            first.feature
        ));
    };
    let made_by = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == first.feature) else {
        return Err(format!(
            "{role}: `{made_by}`, which makes {name}, {}",
            crate::comes_after(doc, earlier, first.feature)
        ));
    };
    let (Some(body), Some(names)) = (&outcome.body, &outcome.names) else {
        return Err(match &outcome.status {
            Status::Ok => format!(
                "{role}: `{made_by}` is a {}, which has no named faces, edges or vertices",
                source.kind.type_name()
            ),
            Status::Suppressed => {
                format!("{role}: `{made_by}`, which makes {name}, is suppressed")
            }
            Status::RolledBack => {
                format!("{role}: `{made_by}`, which makes {name}, is rolled back")
            }
            Status::Failed(_) => {
                format!("{role}: `{made_by}`, which makes {name}, {FIX_IT_FIRST}")
            }
        });
    };
    Ok((body, names, made_by.clone()))
}

pub(crate) fn listed(names: &[PersistentName]) -> String {
    let there: Vec<String> = names.iter().take(12).map(ToString::to_string).collect();
    format!(
        "{}{}",
        if there.is_empty() {
            "none".to_string()
        } else {
            there.join(", ")
        },
        if names.len() > there.len() {
            ", ..."
        } else {
            ""
        }
    )
}

/// The outward normal of a face when it is flat.
fn flat_normal(body: &Body, face: craft3d_topo::FaceId) -> Option<Vec3> {
    let face = body.faces.get(face.0 as usize)?;
    let Surface::Plane(plane) = &face.surface else {
        return None;
    };
    let normal = plane.frame().z();
    Some(if face.reversed { -normal } else { normal })
}

/// The straight line two faces of one body meet in.
fn edge_line(
    doc: &Document,
    earlier: &[FeatureResult],
    between: &[PersistentName; 2],
    tolerance: &Tolerance,
    role: &str,
) -> Result<Line, String> {
    let [a, b] = between;
    for face in between {
        if !face.is_face() {
            return Err(format!(
                "{role}: {face} is not a face; an edge is referred to by the two faces it lies between"
            ));
        }
    }
    if a.feature != b.feature {
        return Err(format!(
            "{role}: {a} and {b} are faces of two features ({} and {}); an edge lies between two faces of one body",
            a.feature, b.feature
        ));
    }
    let (body, names, made_by) = named_body_of(doc, earlier, a, Some(b), role)?;
    let (first, second) = (faces_named(names, a), faces_named(names, b));
    for (face, found) in [(a, &first), (b, &second)] {
        if found.is_empty() {
            return Err(format!(
                "{role}: `{made_by}` no longer has a face {face}; its faces are: {}",
                listed(&names.faces)
            ));
        }
    }
    let edges = edges_between(body, names, a, b);
    let mut line: Option<Line> = None;
    for id in &edges {
        let Some(edge) = body.edges.get(id.0 as usize) else {
            continue;
        };
        let Curve3::Line(curve) = &edge.curve else {
            return Err(format!(
                "{role}: the edge between {a} and {b} of `{made_by}` is a {}, not a straight line",
                edge.curve.kind()
            ));
        };
        let start = curve.origin() + curve.direction() * edge.range.0;
        match &line {
            None => {
                line = Some(Line {
                    origin: start,
                    direction: curve.direction(),
                });
            }
            // Pieces of one edge (a boolean cut it) are one line; edges
            // on different lines do not say which is meant.
            Some(found) => {
                let off = (start - found.origin).cross(found.direction).length();
                let turned = curve.direction().cross(found.direction).length();
                if off > tolerance.linear || turned > tolerance.angular {
                    return Err(format!(
                        "{role}: {a} and {b} of `{made_by}` meet in {} edges that are not on one line",
                        edges.len()
                    ));
                }
            }
        }
    }
    let Some(mut line) = line else {
        return Err(format!(
            "{role}: the faces {a} and {b} of `{made_by}` do not meet in an edge"
        ));
    };
    // Which way the kernel runs an edge is not something a name says.
    // Between two flat faces the line points along the cross product of
    // their outward normals, in the order the faces are given.
    if let (Some(n_a), Some(n_b)) = (
        first.first().and_then(|face| flat_normal(body, *face)),
        second.first().and_then(|face| flat_normal(body, *face)),
    ) && n_a.cross(n_b).dot(line.direction) < 0.0
    {
        line.direction = -line.direction;
    }
    Ok(line)
}

fn axis_ref(
    doc: &Document,
    earlier: &[FeatureResult],
    axis: &AxisRef,
    tolerance: &Tolerance,
    role: &str,
) -> Result<Line, String> {
    match axis {
        AxisRef::World { axis } => Ok(Line {
            origin: Vec3::ZERO,
            direction: match axis {
                WorldAxis::X => Vec3::X,
                WorldAxis::Y => Vec3::Y,
                WorldAxis::Z => Vec3::Z,
            },
        }),
        AxisRef::Edge { between } => edge_line(doc, earlier, between, tolerance, role),
        AxisRef::Datum { feature } => datum_axis(doc, earlier, *feature, role),
    }
}

/// The axis of a face that has one: a cylinder, a cone or a torus.
fn face_axis(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
    role: &str,
) -> Result<Line, String> {
    if !name.is_face() {
        return Err(format!("{role}: {name} is not a face"));
    }
    let (body, names, made_by) = named_body(doc, earlier, name, role)?;
    // The pieces a boolean cut a face into lie on one surface: the first
    // says where the axis is.
    let found = faces_named(names, name)
        .first()
        .and_then(|face| body.faces.get(face.0 as usize));
    let Some(face) = found else {
        return Err(format!(
            "{role}: `{made_by}` no longer has a face {name}; its faces are: {}",
            listed(&names.faces)
        ));
    };
    let frame = match &face.surface {
        Surface::Cylinder(cylinder) => cylinder.frame(),
        Surface::Cone(cone) => cone.frame(),
        Surface::Torus(torus) => torus.frame(),
        other => {
            return Err(format!(
                "{role}: the face {name} is a {} face, which has no axis; a cylinder, a cone and a torus have one",
                other.kind()
            ));
        }
    };
    Ok(Line {
        origin: frame.origin(),
        direction: frame.z(),
    })
}

/// Where a datum axis lies.
pub(crate) fn axis(
    doc: &Document,
    variables: &Evaluation,
    datum: &DatumAxis,
    earlier: &[FeatureResult],
) -> Result<Line, String> {
    let tolerance = document_tolerance(doc)?;
    let line = match &datum.by {
        AxisBy::TwoPoints { points } => {
            let [a, b] = points;
            let a = point_ref(doc, variables, earlier, a, "through.0")?;
            let b = point_ref(doc, variables, earlier, b, "through.1")?;
            let apart = (b - a).length();
            if apart.is_nan() || apart <= tolerance.linear {
                return Err(
                    "through: the two points are the same point; they give no direction"
                        .to_string(),
                );
            }
            Line {
                origin: a,
                direction: (b - a)
                    .normalized()
                    .ok_or_else(|| "through: the points are not finite".to_string())?,
            }
        }
        AxisBy::Edge { between } => edge_line(doc, earlier, between, &tolerance, "edge")?,
        AxisBy::Face { face } => face_axis(doc, earlier, face, "face")?,
        AxisBy::TwoPlanes { planes } => {
            let [a, b] = planes;
            let a = plane_ref(doc, earlier, a, "planes.0")?;
            let b = plane_ref(doc, earlier, b, "planes.1")?;
            let along = a.z().cross(b.z());
            // The sine of the angle between the planes.
            let sine = along.length();
            if sine.is_nan() || sine <= tolerance.angular {
                let apart = (b.origin() - a.origin()).dot(a.z()).abs();
                return Err(if apart <= tolerance.linear {
                    "planes: the two planes are the same plane; they meet everywhere".to_string()
                } else {
                    format!("planes: the two planes are parallel ({apart} apart); they do not meet")
                });
            }
            let direction = along
                .normalized()
                .ok_or_else(|| "planes: the planes are not finite".to_string())?;
            // From the first plane's origin, straight across that plane
            // to the second: the point of the line nearest that origin.
            let across = direction.cross(a.z());
            let slope = across.dot(b.z());
            if slope.abs() <= f64::MIN_POSITIVE {
                return Err("planes: the two planes are parallel; they do not meet".to_string());
            }
            let reach = (b.origin() - a.origin()).dot(b.z()) / slope;
            Line {
                origin: a.origin() + across * reach,
                direction,
            }
        }
    };
    if line.origin.is_finite() && line.direction.is_finite() {
        Ok(line)
    } else {
        Err("the axis is not finite".to_string())
    }
}

/// The position of a vertex, by its persistent name.
fn vertex(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
    role: &str,
) -> Result<Vec3, String> {
    if !name.is_vertex() {
        return Err(format!(
            "{role}: {name} is not a vertex; a vertex is a startVertex, an endVertex, a vertex or a corner"
        ));
    }
    let (body, names, made_by) = named_body(doc, earlier, name, role)?;
    // By its own name, or by the name it kept from the body it came from.
    let found = names
        .vertex_named(name)
        .and_then(|index| body.vertices.get(index));
    match found {
        Some(vertex) => Ok(vertex.point),
        None => Err(format!(
            "{role}: `{made_by}` no longer has a vertex {name}; its vertices are: {}",
            listed(&names.vertices)
        )),
    }
}

fn lengths(
    doc: &Document,
    variables: &Evaluation,
    name: &str,
    exprs: &[Expr; 3],
) -> Result<Vec3, String> {
    let [x, y, z] = exprs;
    let v = Vec3::new(
        length(doc, variables, &format!("{name}.x"), x)?,
        length(doc, variables, &format!("{name}.y"), y)?,
        length(doc, variables, &format!("{name}.z"), z)?,
    );
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{name} is not finite"))
    }
}

fn point_ref(
    doc: &Document,
    variables: &Evaluation,
    earlier: &[FeatureResult],
    point: &PointRef,
    role: &str,
) -> Result<Vec3, String> {
    match point {
        PointRef::At { position } => lengths(doc, variables, role, position),
        PointRef::Vertex { vertex: name } => vertex(doc, earlier, name, role),
        PointRef::Center { between } => center(doc, earlier, between, role),
        PointRef::Datum { feature } => {
            match datum_of(doc, earlier, *feature, role, "datum point")? {
                Datum::Point(point) => Ok(point),
                other => Err(not_a(doc, *feature, role, &other, "datum point")),
            }
        }
    }
}

/// The centre of the circular edge two faces of one body meet in.
fn center(
    doc: &Document,
    earlier: &[FeatureResult],
    between: &[PersistentName; 2],
    role: &str,
) -> Result<Vec3, String> {
    let tolerance = document_tolerance(doc)?;
    let [a, b] = between;
    for face in between {
        if !face.is_face() {
            return Err(format!(
                "{role}: {face} is not a face; an edge is referred to by the two faces it lies between"
            ));
        }
    }
    if a.feature != b.feature {
        return Err(format!(
            "{role}: {a} and {b} are faces of two features ({} and {}); an edge lies between two faces of one body",
            a.feature, b.feature
        ));
    }
    let (body, names, made_by) = named_body_of(doc, earlier, a, Some(b), role)?;
    for face in between {
        if faces_named(names, face).is_empty() {
            return Err(format!(
                "{role}: `{made_by}` no longer has a face {face}; its faces are: {}",
                listed(&names.faces)
            ));
        }
    }
    let edges = edges_between(body, names, a, b);
    let mut found: Option<Vec3> = None;
    for id in &edges {
        let Some(edge) = body.edges.get(id.0 as usize) else {
            continue;
        };
        let Curve3::Circle(circle) = &edge.curve else {
            return Err(format!(
                "{role}: the edge between {a} and {b} of `{made_by}` is a {}, not a circle or an arc of one",
                edge.curve.kind()
            ));
        };
        let centre = circle.frame().origin();
        match found {
            None => found = Some(centre),
            // The arcs a boolean cut a circle into have one centre;
            // circles about different centres do not say which is meant.
            Some(first) if first.distance(centre) > tolerance.linear => {
                return Err(format!(
                    "{role}: {a} and {b} of `{made_by}` meet in {} circular edges that do not have one centre",
                    edges.len()
                ));
            }
            Some(_) => {}
        }
    }
    found.ok_or_else(|| {
        format!("{role}: the faces {a} and {b} of `{made_by}` do not meet in an edge")
    })
}

/// Where a datum point lies.
pub(crate) fn point(
    doc: &Document,
    variables: &Evaluation,
    datum: &DatumPoint,
    earlier: &[FeatureResult],
) -> Result<Vec3, String> {
    let at = point_ref(doc, variables, earlier, &datum.at, "at")?;
    if at.is_finite() {
        Ok(at)
    } else {
        Err("the point is not finite".to_string())
    }
}

/// The helix of the datum helix `id`.
pub(crate) fn datum_helix(
    doc: &Document,
    earlier: &[FeatureResult],
    id: FeatureId,
    role: &str,
) -> Result<Helix, String> {
    match datum_of(doc, earlier, id, role, "datum helix")? {
        Datum::Helix(helix) => Ok(helix),
        other => Err(not_a(doc, id, role, &other, "datum helix")),
    }
}

/// The first of the world's x, y and z axes that is the least along
/// `axis`, made square to it: where a helix given by its radius starts.
pub(crate) fn across(axis: Vec3) -> Vec3 {
    let mut best = Vec3::X;
    for candidate in [Vec3::Y, Vec3::Z] {
        if candidate.dot(axis).abs() < best.dot(axis).abs() {
            best = candidate;
        }
    }
    best - axis * best.dot(axis)
}

/// Where a datum helix lies.
pub(crate) fn helix(
    doc: &Document,
    variables: &Evaluation,
    datum: &DatumHelix,
    earlier: &[FeatureResult],
) -> Result<Helix, String> {
    let tolerance = document_tolerance(doc)?;
    let line = axis_ref(doc, earlier, &datum.axis, &tolerance, "axis")?;
    let axis = line
        .direction
        .normalized()
        .ok_or_else(|| "axis: the axis has no direction".to_string())?;
    let (origin, start, radius) = match &datum.start {
        HelixStart::Radius { radius } => {
            let radius = length(doc, variables, "radius", radius)?;
            if !(radius.is_finite() && radius > tolerance.linear) {
                return Err(format!(
                    "radius: {radius} is not a radius; a helix is further from its axis than the tolerance"
                ));
            }
            (line.origin, across(axis), radius)
        }
        HelixStart::Point { point } => {
            let at = point_ref(doc, variables, earlier, point, "start")?;
            let foot = line.origin + axis * (at - line.origin).dot(axis);
            let radius = at.distance(foot);
            if !(radius.is_finite() && radius > tolerance.linear) {
                return Err(
                    "start: the point is on the axis; a helix starts away from its axis"
                        .to_string(),
                );
            }
            (foot, at - foot, radius)
        }
    };
    let pitch = length(doc, variables, "pitch", &datum.pitch)?;
    if !(pitch.is_finite() && pitch > tolerance.linear) {
        return Err(format!(
            "pitch: {pitch} is not a pitch; a helix advances along its axis by more than the tolerance per turn"
        ));
    }
    let turns = match &datum.extent {
        HelixExtent::Turns { turns } => {
            let turns = crate::number(variables, "turns", turns)?;
            if !(turns.is_finite() && turns > 0.0) {
                return Err(format!("turns: {turns} is not a number of turns above 0"));
            }
            turns
        }
        HelixExtent::Height { height } => {
            let height = length(doc, variables, "height", height)?;
            if !(height.is_finite() && height > tolerance.linear) {
                return Err(format!(
                    "height: {height} is not a height; a helix rises by more than the tolerance"
                ));
            }
            height / pitch
        }
    };
    if turns > craft3d_kernel::helix::MAX_TURNS {
        return Err(format!(
            "the helix has {turns} turns; at most {} are made",
            craft3d_kernel::helix::MAX_TURNS
        ));
    }
    Helix::new(origin, axis, start, radius, pitch, turns, datum.left_handed)
        .map_err(|error| error.to_string())
}

fn document_tolerance(doc: &Document) -> Result<Tolerance, String> {
    Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or_else(|| "the document's tolerance is not positive and finite".to_string())
}

/// Where a datum plane lies.
pub(crate) fn plane(
    doc: &Document,
    variables: &Evaluation,
    datum: &DatumPlane,
    earlier: &[FeatureResult],
) -> Result<Frame, String> {
    let tolerance = document_tolerance(doc)?;
    let degenerate = || "the plane is not finite".to_string();
    match &datum.by {
        PlaneBy::Offset { from, distance } => {
            let frame = plane_ref(doc, earlier, from, "from")?;
            let offset = length(doc, variables, "distance", distance)?;
            if !offset.is_finite() {
                return Err(format!(
                    "distance must be a finite length, but `{}` is {offset}",
                    distance.source()
                ));
            }
            frame
                .at(frame.origin() + frame.z() * offset)
                .ok_or_else(degenerate)
        }
        PlaneBy::Angle {
            from,
            about,
            angle: turn,
        } => {
            let frame = plane_ref(doc, earlier, from, "from")?;
            let axis = axis_ref(doc, earlier, about, &tolerance, "about")?;
            let by = angle(doc, variables, "angle", turn)?;
            if !by.is_finite() {
                return Err(format!(
                    "angle must be a finite angle, but `{}` is {by}",
                    turn.source()
                ));
            }
            // A plane can only be turned about a line it holds, or one
            // beside it: the turned plane goes through the axis.
            let tilt = frame.z().dot(axis.direction).clamp(-1.0, 1.0).asin();
            if tilt.abs() > tolerance.angular {
                return Err(format!(
                    "about: the axis is not parallel to the plane (it makes {} deg with it); a plane is turned about a line in it or beside it",
                    tilt.to_degrees()
                ));
            }
            let turned = Transform::rotation_about(axis.origin, axis.direction, by)
                .ok_or_else(degenerate)?;
            let normal = turned.vector(frame.z());
            let origin = turned.point(frame.origin());
            let origin = origin - normal * (origin - axis.origin).dot(normal);
            Frame::new(origin, normal, turned.vector(frame.x())).ok_or_else(degenerate)
        }
        PlaneBy::ThreePoints { points } => {
            let [a, b, c] = points;
            let a = point_ref(doc, variables, earlier, a, "through.0")?;
            let b = point_ref(doc, variables, earlier, b, "through.1")?;
            let c = point_ref(doc, variables, earlier, c, "through.2")?;
            let (ab, ac) = (b - a, c - a);
            for (first, second, apart) in [
                (0, 1, ab.length()),
                (0, 2, ac.length()),
                (1, 2, (c - b).length()),
            ] {
                // Also refuses what is not a number at all.
                if apart.is_nan() || apart <= tolerance.linear {
                    return Err(format!(
                        "through: points {first} and {second} are the same point; a plane needs three different points"
                    ));
                }
            }
            // How far the third point is from the line through the first
            // two.
            let off = ab.cross(ac).length() / ab.length();
            if off.is_nan() || off <= tolerance.linear {
                return Err(
                    "through: the three points are on one line; they give no plane".to_string(),
                );
            }
            Frame::new(a, ab.cross(ac), ab).ok_or_else(degenerate)
        }
    }
}

/// An edge as a datum refers to it: by the two faces it lies between.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeReference {
    /// The edge's own name, which no other edge of the body has.
    pub name: Option<PersistentName>,
    /// The other names it answers to: those it had in the bodies it came
    /// from.
    pub aliases: Vec<PersistentName>,
    /// The names of the two faces; twice the same for a seam.
    pub between: [PersistentName; 2],
    /// What the edge's curve is: `line`, `circle`, ...
    pub kind: &'static str,
    /// For a straight edge: its two ends.
    pub line: Option<(Vec3, Vec3)>,
    /// For a circular edge: its centre and radius.
    pub circle: Option<(Vec3, f64)>,
}

/// What a datum can be placed by on one body, each with the name it is
/// referred to by.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct References {
    /// The vertices, each by its own name, and where they are.
    pub vertices: Vec<(PersistentName, Vec3)>,
    /// Per vertex of `vertices`, the other names it answers to: those it
    /// had in the bodies it came from (after a boolean).
    pub vertex_aliases: Vec<Vec<PersistentName>>,
    pub edges: Vec<EdgeReference>,
    /// The faces that have an axis (cylinder, cone, torus): the face, a
    /// point on the axis and its direction.
    pub axes: Vec<(PersistentName, Vec3, Vec3)>,
    /// True when a list was cut off at `limit`.
    pub truncated: bool,
}

/// Lists what a datum can be placed by on `body`, at most `limit` of each
/// kind.
pub fn references(body: &Body, names: &crate::Names, limit: usize) -> References {
    let mut out = References::default();
    for (name, vertex) in names.vertices.iter().zip(&body.vertices) {
        if out.vertices.len() >= limit {
            out.truncated = true;
            break;
        }
        out.vertex_aliases
            .push(names.vertex_aliases_of(out.vertices.len()).to_vec());
        out.vertices.push((name.clone(), vertex.point));
    }
    for (index, (edge, faces)) in body
        .edges
        .iter()
        .zip(crate::query::faces_of_edges(body))
        .enumerate()
    {
        let named = |index: usize| {
            faces
                .get(index)
                .or(faces.first())
                .and_then(|face| names.faces.get(face.0 as usize))
                .cloned()
        };
        let (Some(a), Some(b)) = (named(0), named(1)) else {
            continue;
        };
        if out.edges.len() >= limit {
            out.truncated = true;
            break;
        }
        let (line, circle) = match &edge.curve {
            Curve3::Line(line) => {
                let at = |t: f64| line.origin() + line.direction() * t;
                (Some((at(edge.range.0), at(edge.range.1))), None)
            }
            Curve3::Circle(circle) => (None, Some((circle.frame().origin(), circle.radius()))),
            _ => (None, None),
        };
        out.edges.push(EdgeReference {
            name: names.edges.get(index).cloned(),
            aliases: names.edge_aliases_of(index).to_vec(),
            between: [a, b],
            kind: edge.curve.kind(),
            line,
            circle,
        });
    }
    for (name, face) in names.faces.iter().zip(&body.faces) {
        let frame = match &face.surface {
            Surface::Cylinder(surface) => surface.frame(),
            Surface::Cone(surface) => surface.frame(),
            Surface::Torus(surface) => surface.frame(),
            _ => continue,
        };
        if out.axes.len() >= limit {
            out.truncated = true;
            break;
        }
        out.axes.push((name.clone(), frame.origin(), frame.z()));
    }
    out
}
