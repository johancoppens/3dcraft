//! Reference geometry in regeneration: where a datum lies.
//!
//! A datum makes no body. Its result is a plane in space, worked out
//! from what it is placed relative to: a base plane, an earlier datum (by
//! the result of that feature), a face, an edge or a vertex of an earlier
//! body (by its persistent name, against the body of the feature that
//! made it). A reference that points at nothing fails the datum with the
//! reason and the reference it lost; it never resolves to something else.

use crate::query::{edges_between, faces_named};
use crate::{FeatureResult, Status, angle, length};
use craft3d_doc::{
    AxisRef, DatumPlane, Document, Expr, FeatureId, PersistentName, PlaneBy, PlaneRef, PointRef,
    WorldAxis, WorldPlane,
};
use craft3d_expr::Evaluation;
use craft3d_geom::{Curve3, Surface};
use craft3d_math::{Frame, Tolerance, Transform, Vec3};
use craft3d_topo::Body;

/// Where a datum lies, in the document's length unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Datum {
    /// A plane with an origin and axes: z is its normal.
    Plane(Frame),
}

impl Datum {
    /// What the datum is, for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Datum::Plane(_) => "datum plane",
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
            "{role}: `{name}` comes after this feature in the history; move it before (feature.move)"
        ));
    };
    match &outcome.status {
        Status::Ok => {}
        Status::Suppressed => return Err(format!("{role}: `{name}` is suppressed")),
        Status::RolledBack => return Err(format!("{role}: `{name}` is rolled back")),
        Status::Failed(_) => return Err(format!("{role}: `{name}` failed; fix it first")),
    }
    match outcome.datum {
        Some(datum) if datum.kind() == wanted => Ok(datum),
        Some(datum) => Err(format!(
            "{role}: `{name}` is a {}, not a {wanted}",
            datum.kind()
        )),
        None => Err(format!("{role}: `{name}` has no result")),
    }
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
    }
}

fn plane_ref(
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
    let Some(source) = doc.feature(name.feature) else {
        return Err(format!(
            "{role}: there is no feature {} for {name} (was it deleted?)",
            name.feature
        ));
    };
    let made_by = &source.name;
    let Some(outcome) = earlier.iter().find(|r| r.id == name.feature) else {
        return Err(format!(
            "{role}: `{made_by}`, which makes {name}, comes after this feature in the history; move it before (feature.move)"
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
                format!("{role}: `{made_by}`, which makes {name}, failed; fix it first")
            }
        });
    };
    Ok((body, names, made_by.clone()))
}

fn listed(names: &[PersistentName]) -> String {
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
    let (body, names, made_by) = named_body(doc, earlier, a, role)?;
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
    }
}

/// The position of a vertex, by its persistent name.
fn vertex(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
    role: &str,
) -> Result<Vec3, String> {
    use craft3d_doc::NameRole::{EndVertex, StartVertex, Vertex};
    if !matches!(name.root().role, StartVertex | EndVertex | Vertex) {
        return Err(format!(
            "{role}: {name} is not a vertex; a vertex is a startVertex, an endVertex or a vertex"
        ));
    }
    let (body, names, made_by) = named_body(doc, earlier, name, role)?;
    if names.vertices.is_empty() && !body.vertices.is_empty() {
        return Err(format!(
            "{role}: the vertices of `{made_by}` have no names: a boolean and a pattern whose instances are joined name only their faces"
        ));
    }
    let found = names
        .vertices
        .iter()
        .position(|own| own == name)
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
    }
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
