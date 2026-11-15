//! Tip in regeneration: a flat end face of the body of an earlier feature
//! replaced by faces that close the body in one point (`kernel::tip`,
//! docs/design/sweeps.md › *A tip on a body*). How the face is referred
//! to is in [`crate::face_ops`].
//!
//! **Names.** Every face of the body given but the end is still there, on
//! the surface it had, so it is named as a copy of that face
//! (`F9:copy(0)/F2:side(e7)`). A face of the tip is named after the side
//! face it continues, as that side was called in the body given:
//! `F9:tip(0)/F2:side(e7)`. Edges and vertices are named by the faces they
//! lie on (docs/design/names.md): the rails `F9:between[…]` of the two tip
//! faces beside them, the tip `F9:corner[…]` of all of them; an edge or a
//! vertex that was there before answers to the copy of its old name too.

use crate::face_ops::{self, about, explain};
use crate::{FeatureResult, Kept, Names};
use craft3d_doc::{Document, FeatureId, PersistentName, Tip};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::tip::{ROUND_FULLNESS, TipAsk, TipOrigin, tip};
use craft3d_tess::FaceRole;
use craft3d_topo::{Body, FaceId};
use std::collections::BTreeMap;

const NOUN: &str = "a tip";

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    tipped: &Tip,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let source_id = tipped.source();
    let source = face_ops::source(
        doc,
        earlier,
        feature,
        source_id,
        Some(&tipped.face),
        "face",
        NOUN,
    )?;
    let picked = face_ops::pick(
        std::slice::from_ref(&tipped.face),
        "face",
        NOUN,
        source_id,
        &source,
    )
    .map_err(|e| e.replacen("face.0: ", "face: ", 1))?;
    let [end] = picked.as_slice() else {
        return Err(format!(
            "face: {} names {} faces of `{}`; a tip closes one flat end face",
            tipped.face,
            picked.len(),
            source.made_by
        ));
    };
    let length = match &tipped.length {
        Some(expr) => {
            let length = crate::length(doc, variables, "length", expr)?;
            if !(length.is_finite() && length > 0.0) {
                return Err(format!(
                    "length: has to be more than 0, got {length} {}",
                    doc.units.length.symbol()
                ));
            }
            Some(length)
        }
        None => None,
    };
    let at = match &tipped.at {
        Some([x, y]) => Some([
            crate::length(doc, variables, "at.x", x)?,
            crate::length(doc, variables, "at.y", y)?,
        ]),
        None => None,
    };
    let fullness = match &tipped.fullness {
        Some(expr) => {
            let fullness = crate::number(variables, "fullness", expr)?;
            let (least, most) = craft3d_geom::tip::FULLNESS_RANGE;
            if !(fullness >= least && fullness <= most) {
                return Err(format!(
                    "fullness: has to be from {least} to {most} (0.4142 quarter ellipses, 0.5 parabolas, more is blunter), got {fullness}"
                ));
            }
            fullness
        }
        None => ROUND_FULLNESS,
    };
    let ask = TipAsk {
        face: *end,
        length,
        at,
        fullness,
    };
    let unit = doc.units.length.symbol();
    let made = tip(source.body, &ask, &source.tolerance, Some(source.loosest)).map_err(
        |error| match error {
            KernelError::Tip {
                faces,
                edge,
                reason,
            } => {
                // The end itself is said by its name already.
                let others: Vec<FaceId> = faces.into_iter().filter(|f| f != end).collect();
                let at = about(source.body, source.names, &others, edge)
                    .map(|at| format!(" ({at})"))
                    .unwrap_or_default();
                let param = if reason.contains("length") && tipped.length.is_some() {
                    "length"
                } else {
                    "face"
                };
                format!("{param}: {reason}{at}")
            }
            other => explain(other, "length", 1.0, unit, &source),
        },
    )?;
    let names = names(
        feature,
        &made.faces,
        &made.edges,
        &made.vertices,
        &made.body,
        &source,
    )?;
    // The faces of the tip are proven not to cross one another; they are
    // looked at against what is near them, which they might run into
    // (another part of the body beyond the end). Also where the body is
    // not named: the check is of the body, the names only say where.
    let roles: Vec<FaceRole> = made
        .faces
        .iter()
        .map(|origin| match origin {
            TipOrigin::Kept(_) => FaceRole::Known(0),
            TipOrigin::Tip { .. } => FaceRole::New,
        })
        .collect();
    let named = names
        .as_ref()
        .map_or(&[][..], |names| names.faces.as_slice());
    crate::selfcheck::crossing_near(&made.body, named, source.tolerance.linear, &roles).refused(
        "length",
        "tipped body",
        "make the tip shorter, or move what it runs into",
    )?;
    Ok((made.body, names))
}

/// The names of the faces of the tipped body, and of its edges and
/// vertices by its faces; `None` where the body given does not name its
/// faces.
fn names(
    feature: FeatureId,
    faces: &[TipOrigin],
    edges: &[Option<craft3d_topo::EdgeId>],
    vertices: &[Option<craft3d_topo::VertexId>],
    body: &Body,
    source: &face_ops::Source<'_>,
) -> Result<Option<Names>, String> {
    let kept: Vec<Vec<(u64, usize)>> = faces
        .iter()
        .filter_map(|origin| match origin {
            TipOrigin::Kept(face) => Some(vec![(0, face.0 as usize)]),
            TipOrigin::Tip { .. } => None,
        })
        .collect();
    let Some(copies) = Names::copied_faces(feature, &kept, |_| Some(source.names))? else {
        return Ok(None);
    };
    let mut named: Vec<PersistentName> = Vec::with_capacity(faces.len());
    let mut aliases = BTreeMap::new();
    let mut next_kept = 0usize;
    for (index, origin) in faces.iter().enumerate() {
        match origin {
            TipOrigin::Kept(_) => {
                let Some(name) = copies.faces.get(next_kept) else {
                    return Ok(None);
                };
                named.push(name.clone());
                if let Some(also) = copies.aliases.get(&next_kept) {
                    aliases.insert(index, also.clone());
                }
                next_kept += 1;
            }
            TipOrigin::Tip { side } => {
                let Some(side) = source.names.faces.get(side.0 as usize) else {
                    return Ok(None);
                };
                let name = PersistentName::tip(feature, side.clone());
                name.check().map_err(crate::too_long)?;
                named.push(name);
            }
        }
    }
    let mut names = Names {
        faces: named,
        aliases,
        ..Names::default()
    };
    let kept_edges: Kept = edges
        .iter()
        .map(|edge| edge.map(|edge| (0, edge.0 as usize)))
        .collect();
    let kept_vertices: Kept = vertices
        .iter()
        .map(|vertex| vertex.map(|vertex| (0, vertex.0 as usize)))
        .collect();
    names.name_entities(
        feature,
        body,
        source.tolerance.linear,
        &kept_edges,
        &kept_vertices,
        |_| Some(source.names),
    )?;
    Ok(Some(names))
}
