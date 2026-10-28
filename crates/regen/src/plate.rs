//! Plates in regeneration: faces of the body of an earlier feature (a
//! sheet) given a thickness, as a body of its own; the sheet keeps its
//! body (docs/design/plates.md). How a face is referred to is in
//! [`crate::face_ops`].
//!
//! **Names.** The front skin of a face of the sheet is its copy in
//! instance 0 (`F9:copy(0)/F5:face(0)`), the back skin its copy in
//! instance 1; the rim along an edge of the sheet is
//! `F9:rim(h)/F5:face(0)`, after the face the edge bounds and the hash of
//! the edge's name (`PersistentName::rim`). Edges and vertices are named
//! by the faces they lie on.

use crate::face_ops::{self, Source};
use crate::{FeatureResult, Names, angle, length};
use craft3d_doc::{Document, FeatureId, JointKind, PersistentName, PlaneRef, Plate, PlateSide};
use craft3d_expr::Evaluation;
use craft3d_geom::{Plane, Surface};
use craft3d_kernel::KernelError;
use craft3d_kernel::plate::{Joint, Made, Neighbour, Side, plate};
use craft3d_topo::{Body, EdgeId, FaceId};
use std::collections::{BTreeMap, BTreeSet};

const NOUN: &str = "a plate";

fn side(side: PlateSide) -> Side {
    match side {
        PlateSide::In => Side::In,
        PlateSide::Out => Side::Out,
        PlateSide::Mid => Side::Mid,
    }
}

/// One joint as the document gives it, resolved.
struct Resolved<'a> {
    edge: EdgeId,
    kind: JointKind,
    angle: f64,
    near: Option<(Source<'a>, FaceId, f64, Side)>,
    /// The surface an `onto` end lies on.
    onto: Option<Surface>,
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    made: &Plate,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let Some(source_id) = made.source() else {
        return Err("faces: a plate needs at least one face".to_string());
    };
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        made.faces.first(),
        "faces",
        NOUN,
    )?;
    let unit = doc.units.length.symbol();
    let positive = |name: &str, value: f64| -> Result<f64, String> {
        if value.is_finite() && value > source.tolerance.linear {
            Ok(value)
        } else {
            Err(format!(
                "{name}: has to be more than this document's tolerance, got {value} {unit}; the side of the faces it goes on is `side`: in, out or mid"
            ))
        }
    };
    let thickness = positive(
        "thickness",
        length(doc, variables, "thickness", &made.thickness)?,
    )?;
    let picked = face_ops::pick(&made.faces, "faces", NOUN, source_id, &source)?;
    let mut resolved: Vec<Resolved<'_>> = Vec::with_capacity(made.joints.len());
    let mut seen: BTreeSet<EdgeId> = BTreeSet::new();
    for (i, joint) in made.joints.iter().enumerate() {
        let param = format!("joints.{i}");
        let name = &joint.edge;
        if name.feature != source_id || !name.is_edge() {
            return Err(format!(
                "{param}.edge: {name} is not an edge of `{}`, whose faces the plate is made of",
                source.made_by
            ));
        }
        let edge = source
            .names
            .edge_named(name)
            .and_then(|e| u32::try_from(e).ok())
            .map(EdgeId)
            .ok_or_else(|| {
                format!(
                    "{param}.edge: `{}` has no edge {name}; document.edges lists its edges",
                    source.made_by
                )
            })?;
        if !seen.insert(edge) {
            return Err(format!(
                "{param}.edge: {name} has a joint already; an edge has one joint"
            ));
        }
        let angle = match (joint.kind, &joint.angle) {
            (JointKind::Bevel, Some(expr)) => {
                angle(doc, variables, &format!("{param}.angle"), expr)?
            }
            (JointKind::Bevel, None) => {
                return Err(format!(
                    "{param}.angle: a bevel needs its angle from the normal"
                ));
            }
            (_, _) => 0.0,
        };
        let near = if joint.kind.has_neighbour() {
            let Some(with) = &joint.with else {
                return Err(format!(
                    "{param}.with: a {} joint needs the face of the neighbouring sheet the edge runs along",
                    kind_name(joint.kind)
                ));
            };
            let near = face_ops::source(
                doc,
                earlier,
                feature,
                with.feature,
                Some(with),
                &format!("{param}.with"),
                NOUN,
            )?;
            let faces = face_ops::pick(
                std::slice::from_ref(with),
                &format!("{param}.with"),
                NOUN,
                with.feature,
                &near,
            )?;
            let [face] = faces.as_slice() else {
                return Err(format!(
                    "{param}.with: {with} is more than one face; name one"
                ));
            };
            let face = *face;
            let near_thickness = match &joint.thickness {
                Some(expr) => positive(
                    &format!("{param}.thickness"),
                    length(doc, variables, &format!("{param}.thickness"), expr)?,
                )?,
                None => thickness,
            };
            Some((
                near,
                face,
                near_thickness,
                side(joint.side.unwrap_or(made.side)),
            ))
        } else {
            None
        };
        let onto = match (joint.kind, &joint.onto) {
            (JointKind::Onto, Some(plane)) => Some(end_surface(
                doc,
                earlier,
                feature,
                plane,
                &format!("{param}.onto"),
            )?),
            (JointKind::Onto, None) => {
                return Err(format!(
                    "{param}.onto: an onto end needs the surface it lies on: a plane, a datum plane or a face"
                ));
            }
            _ => None,
        };
        resolved.push(Resolved {
            edge,
            kind: joint.kind,
            angle,
            near,
            onto,
        });
    }
    let mut joints: BTreeMap<EdgeId, Joint<'_>> = BTreeMap::new();
    for r in &resolved {
        let neighbour = r.near.as_ref().map(|(near, face, t, s)| Neighbour {
            body: near.body,
            face: *face,
            thickness: *t,
            side: *s,
        });
        let joint = match (r.kind, neighbour) {
            (JointKind::Square, _) => Joint::Square,
            (JointKind::Bevel, _) => Joint::Bevel(r.angle),
            (JointKind::Mitre, Some(n)) => Joint::Mitre(n),
            (JointKind::Touch, Some(n)) => Joint::Touch(n),
            (JointKind::Over, Some(n)) => Joint::Over(n),
            (JointKind::Under, Some(n)) => Joint::Under(n),
            (JointKind::Onto, _) => match &r.onto {
                Some(surface) => Joint::Onto(surface),
                None => return Err("joints: an onto end lost its surface".to_string()),
            },
            (_, None) => return Err("joints: a joint lost its neighbour".to_string()),
        };
        joints.insert(r.edge, joint);
    }
    let built = plate(
        source.body,
        &picked,
        thickness,
        side(made.side),
        &joints,
        &source.tolerance,
    )
    .map_err(|error| explain(error, made, &source))?;
    let names = plate_names(
        feature,
        &built,
        source.body,
        source.names,
        &picked,
        &source.made_by,
        source.tolerance.linear,
    )?;
    face_ops::not_through_itself(
        &built.body,
        &names.faces,
        "thickness",
        "plate",
        "too thick, or a joint that runs into the plate: ",
        &source.tolerance,
    )?;
    Ok((built.body, Some(names)))
}

/// The names of a plate's faces, edges and vertices: a skin is the copy
/// of the sheet's face in instance 0 (front) or 1 (back), a rim
/// `rim(h)/<face>` after the hash of the sheet's edge's name and the face
/// it bounds. `sheet` and `sheet_names` are the faces' body and its names,
/// `picked` the faces plated, `made_by` the sheet's feature for messages.
pub(crate) fn plate_names(
    feature: FeatureId,
    built: &craft3d_kernel::plate::Plate,
    sheet: &Body,
    sheet_names: &Names,
    picked: &[FaceId],
    made_by: &str,
    tolerance: f64,
) -> Result<Names, String> {
    plate_names_with(
        feature,
        built,
        sheet,
        sheet_names,
        &|edge| sheet_names.edge_name(edge),
        picked,
        made_by,
        tolerance,
    )
}

/// [`plate_names`] with the sheet's edges named by `edge_name` (sheet
/// metal names an edge as its counterpart in the part with every corner
/// open).
#[allow(clippy::too_many_arguments)]
pub(crate) fn plate_names_with(
    feature: FeatureId,
    built: &craft3d_kernel::plate::Plate,
    sheet: &Body,
    sheet_names: &Names,
    edge_name: &dyn Fn(usize) -> Option<PersistentName>,
    picked: &[FaceId],
    made_by: &str,
    tolerance: f64,
) -> Result<Names, String> {
    let face_name = |face: FaceId| -> Result<PersistentName, String> {
        sheet_names
            .faces
            .get(face.0 as usize)
            .cloned()
            .ok_or_else(|| "the sheet does not name a face of the plate's".to_string())
    };
    let mut names = Names::default();
    // Per rim's name, the name of the sheet's edge it was made from.
    let mut rims: BTreeMap<PersistentName, PersistentName> = BTreeMap::new();
    for what in &built.faces {
        let name = match what {
            Made::Front(face) | Made::Back(face) => {
                let instance = u64::from(matches!(what, Made::Back(_)));
                PersistentName::copy(feature, instance, face_name(*face)?)
            }
            Made::Rim(edge) => {
                let edge_name = edge_name(edge.0 as usize)
                    .ok_or_else(|| "the sheet does not name an edge of the plate's".to_string())?;
                let face = rim_face(sheet, *edge, picked)
                    .ok_or_else(|| "an edge of the plate bounds none of its faces".to_string())?;
                let rim = PersistentName::rim(feature, &edge_name, face_name(face)?);
                if let Some(other) = rims.insert(rim.clone(), edge_name.clone()) {
                    return Err(format!(
                        "faces: the rims along the edges {other} and {edge_name} of `{made_by}` would both be called {rim}: a rim is named after a 48-bit hash of its edge's name (docs/design/names.md › Rims of a plate), and these two hash alike, which happens to about one pair of edges in 10^14; draw either edge another way (another curve id, another sketch) to tell them apart"
                    ));
                }
                rim
            }
        };
        name.check().map_err(crate::too_long)?;
        names.faces.push(name);
    }
    let none = vec![None; built.body.edges.len()];
    let no_vertices = vec![None; built.body.vertices.len()];
    names.name_entities(feature, &built.body, tolerance, &none, &no_vertices, |_| {
        None
    })?;
    Ok(names)
}

/// The surface an `onto` end lies on: a world or datum plane, or any face
/// of an earlier body by its name (curved too: a stem).
fn end_surface(
    doc: &Document,
    earlier: &[FeatureResult],
    feature: FeatureId,
    plane: &PlaneRef,
    param: &str,
) -> Result<Surface, String> {
    match plane {
        PlaneRef::Face { face } => {
            let near =
                face_ops::source(doc, earlier, feature, face.feature, Some(face), param, NOUN)?;
            let faces =
                face_ops::pick(std::slice::from_ref(face), param, NOUN, face.feature, &near)?;
            let [one] = faces.as_slice() else {
                return Err(format!("{param}: {face} is more than one face; name one"));
            };
            near.body
                .face(*one)
                .map(|f| f.surface.clone())
                .ok_or_else(|| format!("{param}: {face} is not there"))
        }
        PlaneRef::World { .. } | PlaneRef::Datum { .. } => {
            crate::datum::plane_ref(doc, earlier, plane, param)
                .map(|frame| Surface::Plane(Plane::new(frame)))
        }
    }
}

fn kind_name(kind: JointKind) -> &'static str {
    match kind {
        JointKind::Square => "square",
        JointKind::Bevel => "bevel",
        JointKind::Mitre => "mitre",
        JointKind::Touch => "touch",
        JointKind::Over => "over",
        JointKind::Under => "under",
        JointKind::Onto => "onto",
    }
}

/// The face of `faces` whose loops use `edge`.
fn rim_face(body: &Body, edge: EdgeId, faces: &[FaceId]) -> Option<FaceId> {
    faces.iter().copied().find(|f| {
        body.face(*f).is_some_and(|face| {
            face.loops
                .iter()
                .filter_map(|l| body.loop_(*l))
                .any(|l| l.coedges.iter().any(|c| c.edge == edge))
        })
    })
}

/// A kernel refusal in the words of the feature: the joint by its place
/// in the list when it is about an edge that has one.
fn explain(error: KernelError, made: &Plate, source: &Source<'_>) -> String {
    match &error {
        KernelError::Plate {
            edge: Some(edge),
            reason,
        } => {
            let name = source.names.edge_name(edge.0 as usize);
            let place = name.as_ref().and_then(|name| {
                made.joints
                    .iter()
                    .position(|j| source.names.edge_named(&j.edge) == Some(edge.0 as usize))
                    .map(|i| (i, name))
            });
            match (place, name.as_ref()) {
                (Some((i, name)), _) => format!("joints.{i}: along {name}: {reason}"),
                (None, Some(name)) => format!("faces: along {name}: {reason}"),
                (None, None) => format!("faces: {reason}"),
            }
        }
        KernelError::InvalidSize { name, value } => format!("{name}: {value} is not a size"),
        _ => format!("faces: {error}"),
    }
}
