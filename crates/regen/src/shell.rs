//! Shell in regeneration: the body of an earlier feature made hollow.
//! How faces are referred to and how the result is named is in
//! [`crate::face_ops`]: the skin's faces are copies in instance 0, the
//! faces of the hollow copies in instance 1 of the faces they lie behind.

use crate::face_ops::{self, explain};
use crate::{Contributors, FeatureResult, Kept, Names, length};
use craft3d_doc::{Document, FeatureId, FeatureKind, PersistentName, Shell};
use craft3d_expr::Evaluation;
use craft3d_kernel::KernelError;
use craft3d_kernel::shell::{Skin, shell_with_capped};
use craft3d_topo::Body;

const NOUN: &str = "a shell";

fn instance(skin: Skin) -> u64 {
    match skin {
        Skin::Outer => 0,
        Skin::Inner => 1,
    }
}

pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: FeatureId,
    hollowed: &Shell,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let source = face_ops::source(doc, earlier, feature, hollowed.source, None, "source", NOUN)?;
    let thickness = length(doc, variables, "thickness", &hollowed.thickness)?;
    let unit = doc.units.length.symbol();
    if thickness.is_finite() && thickness <= 0.0 {
        return Err(format!(
            "thickness: has to be more than zero, got {thickness} {unit}; the walls are always left inside the faces"
        ));
    }
    if !(thickness.is_finite() && thickness > source.tolerance.linear) {
        return Err(format!(
            "thickness: has to be more than this document's tolerance, got {thickness} {unit}"
        ));
    }
    let open = face_ops::pick(&hollowed.open, "open", NOUN, hollowed.source, &source)?;
    let blends = face_ops::blends(doc, variables, &source);
    let made = shell_with_capped(
        source.body,
        thickness,
        &open,
        &blends,
        &source.tolerance,
        Some(source.loosest),
    )
    .map_err(|error| {
        let thread = made_by_thread(doc, &error, &source);
        let reason = explain(error, "thickness", 1.0, unit, &source);
        match thread {
            Some(thread) => format!(
                "{reason}; the face is made by the thread `{thread}`: put the shell before the thread (shell the body the thread is cut in, then thread the shelled wall), so the shell offsets the plain wall"
            ),
            None => reason,
        }
    })?;
    let contributors: Contributors = made
        .faces
        .iter()
        .map(|(skin, face)| vec![(instance(*skin), face.0 as usize)])
        .collect();
    // The source names both skins: the hollow has its entities.
    let Some(mut names) = Names::copied_faces(feature, &contributors, |_| Some(source.names))?
    else {
        return Ok((made.body, None));
    };
    let kept = |list: &[Option<(Skin, u32)>]| -> Kept {
        list.iter()
            .map(|entry| entry.map(|(skin, id)| (instance(skin), id as usize)))
            .collect()
    };
    let edges: Vec<Option<(Skin, u32)>> = made
        .edges
        .iter()
        .map(|e| e.map(|(skin, edge)| (skin, edge.0)))
        .collect();
    let vertices: Vec<Option<(Skin, u32)>> = made
        .vertices
        .iter()
        .map(|v| v.map(|(skin, vertex)| (skin, vertex.0)))
        .collect();
    names.name_entities(
        feature,
        &made.body,
        source.tolerance.linear,
        &kept(&edges),
        &kept(&vertices),
        |_| Some(source.names),
    )?;
    face_ops::not_through_itself(
        &made.body,
        &names.faces,
        "thickness",
        "shell",
        "too far: ",
        &source.tolerance,
    )?;
    Ok((made.body, Some(names)))
}

/// The name of the thread feature that made a face the kernel's error is
/// about, when one did: a modelled thread's helical faces curve too tightly
/// for most walls to be offset behind them, and shelling first is the
/// remedy.
fn made_by_thread<'a>(
    doc: &'a Document,
    error: &KernelError,
    source: &face_ops::Source<'_>,
) -> Option<&'a str> {
    let faces = match error {
        KernelError::FaceOp { faces, .. } | KernelError::FaceOpTooFar { faces, .. } => faces,
        _ => return None,
    };
    faces.iter().find_map(|face| {
        let mut name: Option<&PersistentName> = source.names.faces.get(face.0 as usize);
        // A copy's or a piece's name says what it is the image of; the
        // chain is as long as the history, and is walked at most that far.
        for _ in 0..=doc.features.len() {
            let at = name?;
            if let Some(feature) = doc.feature(at.feature)
                && matches!(feature.kind, FeatureKind::Thread(_))
            {
                return Some(feature.name.as_str());
            }
            name = at.of.as_deref();
        }
        None
    })
}
