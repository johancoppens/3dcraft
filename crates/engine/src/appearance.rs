//! Which colour and which material each body and each face has: the
//! entries of the document (`Document::bodies`, by the feature that makes
//! a body) carried through the features that make bodies out of bodies.
//!
//! The rule (docs/architecture.md §4.2):
//!
//! - **A body** has the appearance and the material of the feature that
//!   makes it, when that feature has one; otherwise those of the body it
//!   was made from as instance 0 (a boolean's target, the body a fillet
//!   rounds, a pattern repeats or a transform moves), and so on back. So
//!   a cut keeps the colour of the plate it cut, and a fillet does not
//!   turn a red part grey.
//! - **A face** has its body's colour, unless it came into its body as
//!   part of another body that has a colour of its own (decided
//!   2026-10-15). Precisely: walk the face's way back as its persistent
//!   name records it (`F7:copy(1)/F3:side(e2)` came through instance 1 of
//!   F7, the tool, from F3), the feature whose body it is in first. The
//!   first feature on that way that has an appearance of its own gives
//!   the face its colour. A step through an instance whose material the
//!   result does not keep (the tool of a subtraction: its faces bound
//!   the target's material there, turned inside out) ends the way: the
//!   face has the colour of the body at that step, the target's. So a
//!   red boss united with a grey block keeps red faces, and the wall of
//!   a hole a red tool cut into a grey plate is grey, as SolidWorks,
//!   Fusion and Onshape colour it. A face the feature made itself (a
//!   fillet's round) has the body's colour, and so has a face whose way
//!   meets no appearance.
//!
//! Neither changes regeneration: a colour is not part of the history.

use craft3d_doc::{
    Appearance, BooleanOp, Document, Feature, FeatureId, FeatureKind, Material, NameRole,
    PersistentName,
};
use craft3d_regen::Regen;

/// The features whose bodies `feature` was made from, in instance order
/// (for a boolean the target, then the tool), as it was built (with its
/// references followed to the newest body, when they were).
fn sources(doc: &Document, regen: &Regen, feature: FeatureId) -> Vec<FeatureId> {
    let built: Option<&Feature> = regen
        .feature(feature)
        .and_then(|result| result.followed.as_deref());
    built
        .or_else(|| doc.feature(feature))
        .map(|f| f.kind.sources().into_iter().map(|(id, _)| id).collect())
        .unwrap_or_default()
}

/// The feature whose body is instance `instance` of `feature`'s: the
/// source of that number (a boolean's tool for 1), or the one source of
/// a feature that repeats one body.
fn source_of(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    instance: u64,
) -> Option<FeatureId> {
    let sources = sources(doc, regen, feature);
    usize::try_from(instance)
        .ok()
        .and_then(|i| sources.get(i))
        .or_else(|| sources.first())
        .copied()
}

/// Whether the result of `feature` keeps the material of its instance
/// `instance`, so that a face that came through it still bounds that
/// body's material: every instance of every feature but the tools of a
/// subtraction (whose faces bound the target's material, inside out).
fn keeps_material(doc: &Document, regen: &Regen, feature: FeatureId, instance: u64) -> bool {
    let built: Option<&Feature> = regen
        .feature(feature)
        .and_then(|result| result.followed.as_deref());
    match built.or_else(|| doc.feature(feature)).map(|f| &f.kind) {
        Some(FeatureKind::Boolean(b)) => b.op != BooleanOp::Subtract || instance == 0,
        _ => true,
    }
}

/// The first of what `pick` finds on the way from `feature` back through
/// the bodies each was made from as instance 0, `feature` itself first.
fn along_instance_zero<T>(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    pick: impl Fn(FeatureId) -> Option<T>,
) -> Option<T> {
    let mut at = feature;
    // Each step goes to an earlier feature; the cap is for a bad file.
    for _ in 0..=doc.features.len() {
        if let Some(found) = pick(at) {
            return Some(found);
        }
        at = source_of(doc, regen, at, 0)?;
    }
    None
}

fn own_appearance(doc: &Document, feature: FeatureId) -> Option<Appearance> {
    doc.body_properties(feature)?.appearance
}

/// The appearance of the body `feature` makes (see the module
/// documentation); `None` for the default look.
pub fn body_appearance(doc: &Document, regen: &Regen, feature: FeatureId) -> Option<Appearance> {
    along_instance_zero(doc, regen, feature, |at| own_appearance(doc, at))
}

/// The material of the body `feature` makes, and the feature it was given
/// to; `None` when it has none.
pub fn body_material(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
) -> Option<(FeatureId, Material)> {
    along_instance_zero(doc, regen, feature, |at| {
        Some((at, doc.body_properties(at)?.material.clone()?))
    })
}

/// The appearance of one face of the body of `feature`, named `name` there
/// (see the module documentation), when its way meets one; the caller
/// falls back to the body's.
pub fn face_appearance(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    name: &PersistentName,
) -> Option<Appearance> {
    if doc.bodies.is_empty() {
        return None;
    }
    // Where the way stands: a feature, and what is left of the way from
    // its body, as the instances to go through and the name at the end.
    let mut at = feature;
    let mut way: Option<(Vec<u64>, &PersistentName)> = None;
    let mut current = name;
    for _ in 0..=doc.features.len().saturating_mul(2) {
        if let Some(own) = own_appearance(doc, at) {
            return Some(own);
        }
        if way.is_none() {
            if current.role == NameRole::Copy
                && current.feature == at
                && let Some(of) = current.of.as_deref()
            {
                way = Some((current.sources.clone(), of));
            } else {
                // Made here (or named in a way that does not say): the
                // body's colour.
                return body_appearance(doc, regen, at);
            }
        }
        let Some((path, of)) = way.take() else {
            return body_appearance(doc, regen, at);
        };
        let Some((&first, rest)) = path.split_first() else {
            return body_appearance(doc, regen, at);
        };
        if !keeps_material(doc, regen, at, first) {
            // Cut by a tool: the face bounds this body's material.
            return body_appearance(doc, regen, at);
        }
        let Some(next) = source_of(doc, regen, at, first) else {
            return body_appearance(doc, regen, at);
        };
        at = next;
        if path == [0] && next != of.feature {
            // Instance 0 on and on, until the feature the name after `/`
            // is of.
            way = Some((path, of));
        } else if !rest.is_empty() {
            way = Some((rest.to_vec(), of));
        } else {
            current = of;
        }
    }
    body_appearance(doc, regen, feature)
}

/// The appearance of every face of the body of `feature`, in the order of
/// its faces: the body's for every face when the faces have no names.
pub fn face_appearances(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    faces: usize,
) -> Vec<Option<Appearance>> {
    if doc.bodies.is_empty() {
        return vec![None; faces];
    }
    let body = body_appearance(doc, regen, feature);
    let names = regen
        .feature(feature)
        .and_then(|result| result.names.as_deref())
        .map(|names| names.faces.as_slice())
        .unwrap_or_default();
    (0..faces)
        .map(|face| match names.get(face) {
            Some(name) => face_appearance(doc, regen, feature, name).or(body),
            None => body,
        })
        .collect()
}

/// Gives the faces of the body of `feature`, whose ids in a scene's or a
/// picture's mesh start at `first_face_id`, their colours in `colors`.
pub(crate) fn color_faces(
    colors: &mut craft3d_render::FaceColors,
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    first_face_id: u32,
    faces: usize,
) {
    for (face, appearance) in face_appearances(doc, regen, feature, faces)
        .into_iter()
        .enumerate()
    {
        if let (Some(appearance), Ok(face)) = (appearance, u32::try_from(face))
            && let Some(id) = first_face_id.checked_add(face)
        {
            colors.set(id, Some(appearance.color));
        }
    }
}
