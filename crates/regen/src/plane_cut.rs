//! A plane cut in regeneration: the body of an earlier feature cut by
//! planes, one side of each kept (docs/design/plane-cut.md).
//!
//! The planes are applied in the order of the list, each to what the
//! planes before it left, by `craft3d_kernel::halfspace::keep_below`: the
//! boolean intersection with a block made from the body's own box, so
//! nothing is sized by the person. What is left is what lies on the kept
//! side of every plane, whatever the order.
//!
//! Names. A face that is a piece of a face of the target is its copy in
//! instance 0 (`F9:copy(0)/F1:face(1)`), as after every feature that
//! takes a body. A face a plane leaves is named by that plane's id,
//! `F9:cut(3)`: the id is stored with the plane and never given to
//! another, so the name stays with its plane when planes are added,
//! deleted or put in another order. Where the boolean merged a piece of a
//! plane with a face of the body that lies in it, the face keeps the
//! body's name and answers to the plane's as an alias. Edges and vertices
//! are named by the faces they lie on, and one the target already had
//! also answers to the copy of the name it had there.

use crate::derive::{direction, lengths, source_body};
use crate::selfcheck::{crossing_near, roles_of};
use crate::{Contributors, CutReport, FeatureResult, Kept, Names, length, too_long};
use craft3d_doc::{
    CutPlane, CutPlaneBy, CutSide, Document, FeatureId, NameRole, PersistentName, PlaneCut,
};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::boolean::{
    BooleanOptions, BooleanResult, EdgeSource, FaceSource, Operand, VertexSource,
};
use craft3d_kernel::halfspace::{self, keep_below};
use craft3d_math::{Frame, Tolerance};
use craft3d_topo::{Body, BodyKind};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// What is known of the body as the planes are applied: per face its
/// name and the names it answers to besides (when the target names its
/// faces), the ids of the planes it has a piece of, and per edge and
/// vertex the entity of the target it is, when it is one.
struct Running {
    faces: Option<Vec<PersistentName>>,
    aliases: BTreeMap<usize, Vec<PersistentName>>,
    planes: Vec<Vec<u32>>,
    edges: Vec<Option<usize>>,
    vertices: Vec<Option<usize>>,
}

/// The name of the face plane `id` of `feature` leaves.
fn cut_name(feature: FeatureId, id: u32) -> PersistentName {
    PersistentName::new(feature, NameRole::Cut, vec![u64::from(id)])
}

impl Running {
    /// The target as it is before any plane: every face its copy in
    /// instance 0, every edge and vertex itself.
    fn start(feature: FeatureId, body: &Body, names: Option<&Names>) -> Result<Running, String> {
        let whole: Contributors = (0..body.faces.len()).map(|i| vec![(0, i)]).collect();
        let copied = Names::copied_faces(feature, &whole, |_| names)?;
        let (faces, aliases) = match copied {
            Some(copied) => (Some(copied.faces), copied.aliases),
            None => (None, BTreeMap::new()),
        };
        Ok(Running {
            faces,
            aliases,
            planes: vec![Vec::new(); body.faces.len()],
            edges: (0..body.edges.len()).map(Some).collect(),
            vertices: (0..body.vertices.len()).map(Some).collect(),
        })
    }

    /// What face `face` of the body as it is now is called, for a reason.
    fn face_name(&self, face: usize) -> String {
        self.faces
            .as_ref()
            .and_then(|faces| faces.get(face))
            .map_or_else(|| format!("face {face} of the body"), ToString::to_string)
    }

    /// The same for the body `result` is: its faces named after what they
    /// have a piece of (the first of the body's faces, else the plane
    /// `id`), answering to the names of the rest; its edges and vertices
    /// followed back to the target's.
    fn after(
        &self,
        feature: FeatureId,
        id: u32,
        result: &BooleanResult,
    ) -> Result<Running, String> {
        let lost = || {
            craft3d_math::bug_text!("a plane cut lost track of a face of the body it cuts")
                .to_string()
        };
        let count = result.body.faces.len();
        let mut faces = self.faces.as_ref().map(|_| Vec::with_capacity(count));
        let mut aliases = BTreeMap::new();
        let mut planes = Vec::with_capacity(count);
        for index in 0..count {
            let own: Vec<FaceSource> = result.faces.get(index).copied().into_iter().collect();
            let sources = match result.merged.get(index) {
                Some(merged) if !merged.is_empty() => merged.as_slice(),
                _ => own.as_slice(),
            };
            let mut ids: Vec<u32> = Vec::new();
            let mut all: Vec<PersistentName> = Vec::new();
            let mut inherited: Vec<PersistentName> = Vec::new();
            for source in sources {
                let there = source.face.0 as usize;
                match source.operand {
                    Operand::A => {
                        for plane in self.planes.get(there).ok_or_else(lost)? {
                            if !ids.contains(plane) {
                                ids.push(*plane);
                            }
                        }
                        if let Some(named) = &self.faces {
                            let name = named.get(there).ok_or_else(lost)?;
                            if !all.contains(name) {
                                all.push(name.clone());
                            }
                            inherited
                                .extend(self.aliases.get(&there).into_iter().flatten().cloned());
                        }
                    }
                    Operand::B => {
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                        let name = cut_name(feature, id);
                        if self.faces.is_some() && !all.contains(&name) {
                            all.push(name);
                        }
                    }
                }
            }
            for alias in inherited {
                if !all.contains(&alias) {
                    all.push(alias);
                }
            }
            if let Some(named) = &mut faces {
                let mut rest = all.into_iter();
                named.push(rest.next().ok_or_else(lost)?);
                let others: Vec<PersistentName> = rest.collect();
                if !others.is_empty() {
                    aliases.insert(index, others);
                }
            }
            planes.push(ids);
        }
        let edges = result
            .edges
            .iter()
            .map(|source| match source {
                EdgeSource::Kept {
                    operand: Operand::A,
                    edge,
                } => self.edges.get(edge.0 as usize).copied().flatten(),
                _ => None,
            })
            .collect();
        let vertices = result
            .vertices
            .iter()
            .map(|source| match source {
                VertexSource::Kept {
                    operand: Operand::A,
                    vertex,
                } => self.vertices.get(vertex.0 as usize).copied().flatten(),
                _ => None,
            })
            .collect();
        Ok(Running {
            faces,
            aliases,
            planes,
            edges,
            vertices,
        })
    }
}

/// The frame of plane `index` as the kernel takes it: its origin on the
/// plane moved by its offset, its z axis pointing to the side that is cut
/// away. For `side: above` that is the plane seen from its other side (x
/// stays, y and z turn round).
fn frame_of(
    doc: &Document,
    variables: &Evaluation,
    earlier: &[FeatureResult],
    index: usize,
    plane: &CutPlane,
) -> Result<Frame, String> {
    let role = format!("planes.{index}");
    let frame = match &plane.by {
        CutPlaneBy::Plane { plane } => crate::datum::plane_ref(doc, earlier, plane, &role)?,
        CutPlaneBy::PointNormal { point, normal } => {
            let origin = lengths(doc, variables, &format!("{role}.point"), point)?;
            let z = direction(doc, variables, &format!("{role}.normal"), normal)?;
            Frame::from_z(origin, z).ok_or_else(|| format!("{role}: the plane is not finite"))?
        }
    };
    let offset = length(doc, variables, &format!("{role}.offset"), &plane.offset)?;
    let origin = frame.origin() + frame.z() * offset;
    match plane.side {
        CutSide::Below => frame.at(origin),
        CutSide::Above => frame.reversed_at(origin),
    }
    .ok_or_else(|| format!("{role}.offset: the plane is not finite"))
}

/// Why plane `index` (of id `id`) could not be applied, in the feature's
/// words: the parameter first, then the plane, the faces by their names.
fn explain(
    error: KernelError,
    feature: FeatureId,
    target: &str,
    index: usize,
    plane: &CutPlane,
    running: &Running,
    tolerance: &Tolerance,
) -> String {
    let id = plane.id;
    let role = format!("planes.{index}");
    let named = |operand: Operand, face: craft3d_topo::FaceId| match operand {
        Operand::A => running.face_name(face.0 as usize),
        Operand::B => cut_name(feature, id).to_string(),
    };
    // A state a plane further down the list would cut away is refused
    // all the same: each plane is a whole boolean.
    let later =
        "a plane further down the list that takes that place away can be put before this one";
    match error {
        KernelError::EmptyResult { .. } => {
            let (kept, cut) = match plane.side {
                CutSide::Below => ("below", "above"),
                CutSide::Above => ("above", "below"),
            };
            let before = if index == 0 {
                format!("`{target}`")
            } else {
                format!("what the planes before it leave of `{target}`")
            };
            format!(
                "{role}: plane {id} leaves nothing: {before} lies wholly {cut} it, the side that is cut away (it keeps what lies {kept}); turn its side or move it"
            )
        }
        KernelError::Unsupported { reason, faces, .. }
            if reason.ends_with("(at the edge of tolerance)") =>
        {
            let listed: Vec<String> = faces
                .iter()
                .map(|&(operand, face)| named(operand, face))
                .collect();
            let at = if listed.is_empty() {
                String::new()
            } else {
                format!(" ({})", listed.join(", "))
            };
            let tol = tolerance.linear;
            format!(
                "{role}: plane {id}: {reason}{at}. At the document's tolerance of {tol:e}, faces or points between half of it ({:e}) and twice it ({:e}) apart are neither one place nor two, and nothing is snapped: move the plane by more than the tolerance, or give planes that are meant to meet in one point the same point; {later}",
                0.5 * tol,
                2.0 * tol
            )
        }
        KernelError::ZeroThickness { at, faces, .. } => {
            let [a, b] = faces.map(|(operand, face)| named(operand, face));
            format!(
                "{role}: plane {id} would leave parts that touch at ({}, {}, {}), along an edge or in a point, at {a} and {b}, and no solid can be that thin; move the plane by more than the tolerance; {later}",
                at.x, at.y, at.z
            )
        }
        KernelError::Unsupported { reason, faces, .. } => {
            let listed: Vec<String> = faces
                .iter()
                .map(|&(operand, face)| named(operand, face))
                .collect();
            let at = if listed.is_empty() {
                String::new()
            } else {
                format!(": {}", listed.join(", "))
            };
            format!("{role}: plane {id}: the cut is not built: {reason}{at}")
        }
        other => format!("{role}: plane {id}: {other}"),
    }
}

/// The body of the target cut by the planes, its names, and per plane
/// what it did ([`CutReport`]).
pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    cut: &PlaneCut,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>, Vec<CutReport>), String> {
    if let Some(fault) = cut.fault() {
        return Err(format!("planes: {fault}"));
    }
    if cut.target == feature {
        return Err("target: a body cannot be cut from itself".to_string());
    }
    let target = source_body(doc, earlier, cut.target, "target")?;
    let Some(whole) = &target.body else {
        return Err("target: it has no body".to_string());
    };
    let target_name = doc
        .feature(cut.target)
        .map_or_else(|| cut.target.to_string(), |f| f.name.clone());
    if whole.kind != BodyKind::Solid {
        return Err(format!(
            "target: `{target_name}` is a sheet, which has no inside to keep a side of: a plane cut takes a solid (a sheet is trimmed by a solid with feature.boolean)"
        ));
    }
    let tolerance = Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or("the document's tolerance is not positive and finite".to_string())?;
    // The document's cap: a body with loose entities is worked on at its
    // largest tolerance up to it, as by any boolean.
    let options = BooleanOptions {
        loosest: Some(doc.tolerance.loosest_linear()),
        ..BooleanOptions::default()
    };
    let names = target.names.as_deref();
    let mut running = Running::start(feature, whole, names)?;
    let mut body: Cow<'_, Body> = Cow::Borrowed(whole.as_ref());
    // Per plane: its frame, and whether it took anything away at its turn.
    let mut applied: Vec<(Frame, bool)> = Vec::with_capacity(cut.planes.len());
    for (index, plane) in cut.planes.iter().enumerate() {
        let frame = frame_of(doc, variables, earlier, index, plane)?;
        match keep_below(&body, &frame, &tolerance, &options) {
            Ok(halfspace::Kept::Whole) => applied.push((frame, false)),
            Ok(halfspace::Kept::Cut(result)) => {
                // A piece of the plane is in the body, or lumps wholly
                // beyond the plane are gone from it: either way the body
                // is another.
                running = running.after(feature, plane.id, &result)?;
                body = Cow::Owned(result.body);
                applied.push((frame, true));
            }
            Err(error) => {
                return Err(explain(
                    error,
                    feature,
                    &target_name,
                    index,
                    plane,
                    &running,
                    &tolerance,
                ));
            }
        }
    }
    let reports: Vec<CutReport> = cut
        .planes
        .iter()
        .zip(&applied)
        .map(|(plane, &(frame, cuts))| CutReport {
            id: plane.id,
            cuts,
            faces: running
                .planes
                .iter()
                .filter(|ids| ids.contains(&plane.id))
                .count(),
            frame,
        })
        .collect();
    let cut_any = matches!(body, Cow::Owned(_));
    let body = body.into_owned();
    // A face with a piece of a plane is new; one that is a piece of a face
    // of the target alone was looked at with the target.
    let contributors: Contributors = running
        .planes
        .iter()
        .map(|ids| {
            if ids.is_empty() {
                vec![(0, 0)]
            } else {
                vec![(1, 0)]
            }
        })
        .collect();
    let Running {
        faces,
        aliases,
        edges,
        vertices,
        ..
    } = running;
    let names = match faces {
        Some(faces) => {
            for name in faces.iter().chain(aliases.values().flatten()) {
                name.check().map_err(too_long)?;
            }
            let mut made = Names {
                faces,
                aliases,
                ..Names::default()
            };
            let kept = |list: &[Option<usize>]| -> Kept {
                list.iter()
                    .map(|from| from.map(|entity| (0, entity)))
                    .collect()
            };
            made.name_entities(
                feature,
                &body,
                tolerance.linear,
                &kept(&edges),
                &kept(&vertices),
                |_| names,
            )?;
            Some(made)
        }
        None => None,
    };
    if cut_any {
        let faces = names
            .as_ref()
            .map_or(&[][..], |names| names.faces.as_slice());
        crossing_near(&body, faces, tolerance.linear, &roles_of(&contributors, &[1])).refused(
            "planes",
            "cut body",
            "a plane meets the body there in a way the cut did not find, and the result is refused rather than handed on",
        )?;
    }
    Ok((body, names, reports))
}
