//! Which colour and which material each body and each face has: the
//! entries of the document (`Document::bodies`, by the feature that makes
//! a body) carried through the features that make bodies out of bodies.
//!
//! The rule (docs/design/document.md):
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
//! - **A face given a colour of its own** (`Document::faces`, by the
//!   feature whose body has it and its name there) has it, before its
//!   body's at that feature; the faces later features make of it (copies
//!   through a boolean, a pattern, a transform, a blend; the pieces of a
//!   face cut in two, which share its name) find it on the same way back,
//!   which looks at each step for an entry of the face's name in that
//!   step's body before the body's own appearance. So the nearest
//!   feature on the way wins, a face over its body at the same feature.
//!
//! Neither changes regeneration: a colour is not part of the history.

use craft3d_doc::{
    Appearance, BooleanOp, Document, Feature, FeatureId, FeatureKind, Material, NameRole,
    PersistentName,
};
use craft3d_regen::Regen;
use std::collections::BTreeMap;

/// What the walks below ask of the document and the regeneration, by
/// feature, found once per walk: each is a function of the document and
/// the regeneration, so what is remembered is what asking again gives.
/// A body of many faces asks the same questions for every face (a scene
/// colours tens of thousands of faces), and each asks for a feature's
/// result and definition, which the regeneration and the document find
/// by a search through the features.
#[derive(Default)]
struct Memo {
    /// [`sources`] of a feature.
    sources: BTreeMap<FeatureId, Vec<FeatureId>>,
    /// Whether a feature's definition is a subtraction, for
    /// [`keeps_material`].
    subtracts: BTreeMap<FeatureId, bool>,
    /// [`body_appearance`] of a feature.
    bodies: BTreeMap<FeatureId, Option<Appearance>>,
}

/// The definition of `feature` as it was built (with its references
/// followed to the newest body, when they were), else as it stands.
fn built<'a>(doc: &'a Document, regen: &'a Regen, feature: FeatureId) -> Option<&'a Feature> {
    regen
        .feature(feature)
        .and_then(|result| result.followed.as_deref())
        .or_else(|| doc.feature(feature))
}

/// The features whose bodies `feature` was made from, in instance order
/// (for a boolean the target, then the tool), as it was built.
fn sources(doc: &Document, regen: &Regen, memo: &mut Memo, feature: FeatureId) -> Vec<FeatureId> {
    if let Some(found) = memo.sources.get(&feature) {
        return found.clone();
    }
    let found: Vec<FeatureId> = built(doc, regen, feature)
        .map(|f| f.kind.sources().into_iter().map(|(id, _)| id).collect())
        .unwrap_or_default();
    memo.sources.insert(feature, found.clone());
    found
}

/// The feature whose body is instance `instance` of `feature`'s: the
/// source of that number (a boolean's tool for 1), or the one source of
/// a feature that repeats one body.
fn source_of(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    instance: u64,
) -> Option<FeatureId> {
    let sources = sources(doc, regen, memo, feature);
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
fn keeps_material(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    instance: u64,
) -> bool {
    let subtracts = match memo.subtracts.get(&feature) {
        Some(&subtracts) => subtracts,
        None => {
            let subtracts = matches!(
                built(doc, regen, feature).map(|f| &f.kind),
                Some(FeatureKind::Boolean(b)) if b.op == BooleanOp::Subtract
            );
            memo.subtracts.insert(feature, subtracts);
            subtracts
        }
    };
    !subtracts || instance == 0
}

/// The first of what `pick` finds on the way from `feature` back through
/// the bodies each was made from as instance 0, `feature` itself first.
fn along_instance_zero<T>(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    pick: impl Fn(FeatureId) -> Option<T>,
) -> Option<T> {
    let mut at = feature;
    // Each step goes to an earlier feature; the cap is for a bad file.
    for _ in 0..=doc.features.len() {
        if let Some(found) = pick(at) {
            return Some(found);
        }
        at = source_of(doc, regen, memo, at, 0)?;
    }
    None
}

fn own_appearance(doc: &Document, feature: FeatureId) -> Option<Appearance> {
    doc.body_properties(feature)?.appearance
}

/// The colour given to a face of the body of `at` whose name there is the
/// copy in `at` along `path` of `of` (`way`), or `name` itself.
fn own_face_appearance(
    doc: &Document,
    at: FeatureId,
    way: Option<(&[u64], &PersistentName)>,
    name: &PersistentName,
) -> Option<Appearance> {
    let entries = doc.face_properties(at);
    if entries.is_empty() {
        return None;
    }
    match way {
        Some((path, of)) => entries
            .iter()
            .find(|e| {
                e.face.feature == at
                    && e.face.role == NameRole::Copy
                    && e.face.by.is_none()
                    && e.face.sources == path
                    && e.face.of.as_deref() == Some(of)
            })
            .map(|e| e.appearance),
        None => doc.face_appearance(at, name),
    }
}

/// The appearance of the body `feature` makes (see the module
/// documentation); `None` for the default look.
pub fn body_appearance(doc: &Document, regen: &Regen, feature: FeatureId) -> Option<Appearance> {
    body_appearance_with(doc, regen, &mut Memo::default(), feature)
}

fn body_appearance_with(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
) -> Option<Appearance> {
    if let Some(&found) = memo.bodies.get(&feature) {
        return found;
    }
    let found = along_instance_zero(doc, regen, memo, feature, |at| own_appearance(doc, at));
    memo.bodies.insert(feature, found);
    found
}

/// The material of the body `feature` makes, and the feature it was given
/// to; `None` when it has none.
pub fn body_material(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
) -> Option<(FeatureId, Material)> {
    along_instance_zero(doc, regen, &mut Memo::default(), feature, |at| {
        Some((at, doc.body_properties(at)?.material.clone()?))
    })
}

/// The appearance of one face of the body of `feature`, named `name` there
/// (see the module documentation), when its way meets one; the caller
/// falls back to the body's.
fn face_appearance(
    doc: &Document,
    regen: &Regen,
    memo: &mut Memo,
    feature: FeatureId,
    name: &PersistentName,
) -> Option<Appearance> {
    if doc.bodies.is_empty() && doc.faces.is_empty() {
        return None;
    }
    // Where the way stands: a feature, and what is left of the way from
    // its body, as the instances to go through and the name at the end.
    let mut at = feature;
    let mut way: Option<(Vec<u64>, &PersistentName)> = None;
    let mut current = name;
    for _ in 0..=doc.features.len().saturating_mul(2) {
        let here = way.as_ref().map(|(path, of)| (path.as_slice(), *of));
        if let Some(own) = own_face_appearance(doc, at, here, current) {
            return Some(own);
        }
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
                return body_appearance_with(doc, regen, memo, at);
            }
        }
        let Some((path, of)) = way.take() else {
            return body_appearance_with(doc, regen, memo, at);
        };
        let Some((&first, rest)) = path.split_first() else {
            return body_appearance_with(doc, regen, memo, at);
        };
        if !keeps_material(doc, regen, memo, at, first) {
            // Cut by a tool: the face bounds this body's material.
            return body_appearance_with(doc, regen, memo, at);
        }
        let Some(next) = source_of(doc, regen, memo, at, first) else {
            return body_appearance_with(doc, regen, memo, at);
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
    body_appearance_with(doc, regen, memo, feature)
}

/// The appearance of every face of the body of `feature`, in the order of
/// its faces: the body's for every face when the faces have no names.
pub fn face_appearances(
    doc: &Document,
    regen: &Regen,
    feature: FeatureId,
    faces: usize,
) -> Vec<Option<Appearance>> {
    if doc.bodies.is_empty() && doc.faces.is_empty() {
        return vec![None; faces];
    }
    let mut memo = Memo::default();
    let body = body_appearance_with(doc, regen, &mut memo, feature);
    let names = regen
        .feature(feature)
        .and_then(|result| result.names.as_deref())
        .map(|names| names.faces.as_slice())
        .unwrap_or_default();
    // A face whose name does not say it came in through an instance of
    // this feature has the body's colour, as the walk would find at its
    // first step (and when the feature has an appearance of its own,
    // every face has that: the body's).
    let walks = own_appearance(doc, feature).is_none();
    (0..faces)
        .map(|face| match names.get(face) {
            Some(name) => {
                if let Some(own) = doc.face_appearance(feature, name) {
                    Some(own)
                } else if walks && name.role == NameRole::Copy && name.feature == feature {
                    face_appearance(doc, regen, &mut memo, feature, name).or(body)
                } else {
                    body
                }
            }
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

/// A face colour whose face is not in its body: the feature built a body
/// with named faces and none of them is called that (by its own name or
/// an alias). It is kept (undoing the edit that took the face brings the
/// colour back) and colours nothing; never another face.
#[derive(Clone, Debug, PartialEq)]
pub struct LostFace {
    pub feature: FeatureId,
    pub face: PersistentName,
}

/// The face colours of the document whose faces are gone (see
/// [`LostFace`]). One of a feature that did not build (failed,
/// suppressed, rolled back) is not lost: it waits for the body.
pub fn lost_faces(doc: &Document, regen: &Regen) -> Vec<LostFace> {
    let mut lost = Vec::new();
    let mut at = 0;
    while let Some(first) = doc.faces.get(at) {
        let feature = first.feature;
        let entries = doc.face_properties(feature);
        at = at.saturating_add(entries.len().max(1));
        let Some(names) = regen
            .feature(feature)
            .filter(|result| result.body.is_some())
            .and_then(|result| result.names.as_deref())
        else {
            continue;
        };
        for entry in entries {
            if craft3d_regen::faces_named(names, &entry.face).is_empty() {
                lost.push(LostFace {
                    feature,
                    face: entry.face.clone(),
                });
            }
        }
    }
    lost
}
