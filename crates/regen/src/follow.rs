//! References that follow a body to its newest form.
//!
//! A feature that takes faces or edges of a body and makes its own body
//! out of it (a chamfer, a fillet, an offset face, a delete face, a shell,
//! a thread)
//! may name them as an earlier body had them: `F2:face(0)` where the body
//! of `F2` already went into a fillet `F10` and a chamfer `F11`. Such a
//! name is followed through the features that took the body, each in
//! turn, up to the newest body before the feature in the history: every
//! feature that makes its body out of another names the faces it keeps as
//! copies (`F10:copy(0)/F2:face(0)`, a boolean's tool `F3:copy(1)/…`), so
//! one step is finding the copy of the name in the taker's names. The
//! feature is then built as if it had been given the names in full
//! (`F11:copy(0)/F10:copy(0)/F2:face(0)`), and those are what it means:
//! a short name is an abbreviation of exactly one long one.
//!
//! The names stay in the document as they were given and are followed at
//! every regeneration (docs/decisions.md): a feature inserted between
//! that takes the body is followed through too, and one taken out is no
//! longer. A feature that failed, is suppressed or is rolled back took no
//! body, so it is not followed through, as everywhere else.
//!
//! What cannot be followed fails the feature with the reason: a face the
//! taker has no copy of (a delete face took it away, a boolean cut it
//! off) names the taker; a face the taker has several copies of (the
//! instances of a pattern, the skin and the hollow of a shell) lists them.
//! A face a boolean cut in pieces is one copy, which names every piece,
//! as the long name does.
//!
//! References that do not take the body (a sketch on a face, a datum, an
//! extrusion up to a face) are not followed: they keep meaning the face
//! of the body they name, as they did before (docs/decisions.md).

use crate::datum::listed;
use crate::sketch::same_plane;
use crate::{FeatureResult, Names};
use craft3d_doc::{Document, Feature, FeatureId, FeatureKind, NameRole, PersistentName};
use std::collections::BTreeSet;

fn called(doc: &Document, id: FeatureId) -> String {
    doc.feature(id)
        .map_or_else(|| id.to_string(), |f| f.name.clone())
}

/// The features the body of `start` went into, one after the other, up to
/// the one whose body is its own still: the chain of consumption at this
/// point in the history (`earlier` are the results before it).
pub(crate) fn chain(earlier: &[FeatureResult], start: FeatureId) -> Vec<FeatureId> {
    let mut out: Vec<FeatureId> = Vec::new();
    let mut at = start;
    // Each feature takes at most one body and comes later than it, so
    // the chain is no longer than the history; bounded all the same.
    for _ in 0..earlier.len() {
        let Some(taker) = earlier
            .iter()
            .find(|r| r.id == at)
            .and_then(|r| r.consumed_by)
        else {
            break;
        };
        if taker == start || out.contains(&taker) {
            break;
        }
        out.push(taker);
        at = taker;
    }
    out
}

/// The feature whose body the body of `feature` is now, at the end of
/// `results`: the last of its chain of consumption, or `feature` itself.
pub fn newest_body(results: &[FeatureResult], feature: FeatureId) -> FeatureId {
    chain(results, feature).last().copied().unwrap_or(feature)
}

/// Whether some face of `names` answers to `name`, by its own name or an
/// alias; a cap also when only the outline it was named after changed.
fn has_face(names: &Names, name: &PersistentName) -> bool {
    names.all_face_names().any(|face| same_plane(face, name))
}

/// The copies the taker `taker` made of the face `name`: the instances
/// `k` for which a face of `names` answers to `taker:copy(k)/name`.
fn copies(names: &Names, taker: FeatureId, name: &PersistentName) -> BTreeSet<u64> {
    names
        .all_face_names()
        .filter(|face| face.feature == taker && face.role == NameRole::Copy)
        .filter(|face| face.of.as_deref().is_some_and(|of| same_plane(of, name)))
        .filter_map(|face| face.sources.first().copied())
        .collect()
}

/// The name, in the newest body of its chain, of the face `name` of an
/// earlier body; `None` when that body is its own still (nothing to
/// follow). `role` is the parameter, as reasons start with it.
pub(crate) fn face(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
    role: &str,
) -> Result<Option<PersistentName>, String> {
    let takers = chain(earlier, name.feature);
    if takers.is_empty() || !name.is_face() {
        return Ok(None);
    }
    let start = called(doc, name.feature);
    // The face has to be one of the body it says it is of: otherwise the
    // name is wrong, not lost on the way.
    let made = earlier
        .iter()
        .find(|r| r.id == name.feature)
        .and_then(|r| r.names.as_ref());
    if let Some(names) = made
        && !has_face(names, name)
    {
        return Err(format!(
            "{role}: `{start}` has no face {name}; its faces are: {}",
            listed(&names.faces)
        ));
    }
    let mut current = name.clone();
    let mut from = start;
    for taker in takers {
        let by = called(doc, taker);
        let kind = doc.feature(taker).map_or("feature", |f| f.kind.type_name());
        let Some(names) = earlier
            .iter()
            .find(|r| r.id == taker)
            .and_then(|r| r.names.as_ref())
        else {
            return Err(format!(
                "{role}: the body of `{from}` went into `{by}`, a {kind} that does not name its faces, so {name} cannot be followed there"
            ));
        };
        let found = copies(names, taker, &current);
        let mut found = found.into_iter();
        match (found.next(), found.next()) {
            (None, _) => {
                return Err(format!(
                    "{role}: the face {name} is gone: the body of `{from}` went into `{by}`, a {kind}, which left no face of it; its faces are: {}",
                    listed(&names.faces)
                ));
            }
            (Some(k), None) => current = PersistentName::copy(taker, k, current),
            (Some(first), Some(second)) => {
                let all: Vec<String> = [first, second]
                    .into_iter()
                    .chain(found)
                    .map(|k| PersistentName::copy(taker, k, current.clone()).to_string())
                    .collect();
                return Err(format!(
                    "{role}: the face {name} is more than one face after `{by}`, a {kind} the body of `{from}` went into: {}; give one of these names",
                    all.join(", ")
                ));
            }
        }
        from = by;
    }
    Ok(Some(current))
}

/// Follows every name in `list`, each as `{param}.{i}`; `true` when one
/// changed.
fn faces(
    doc: &Document,
    earlier: &[FeatureResult],
    list: &mut [PersistentName],
    param: &str,
) -> Result<bool, String> {
    let mut changed = false;
    for (index, name) in list.iter_mut().enumerate() {
        if let Some(followed) = face(doc, earlier, name, &format!("{param}.{index}"))? {
            *name = followed;
            changed = true;
        }
    }
    Ok(changed)
}

/// The same for the pairs of faces that blends refer to edges by.
fn pairs(
    doc: &Document,
    earlier: &[FeatureResult],
    list: &mut [[PersistentName; 2]],
) -> Result<bool, String> {
    let mut changed = false;
    for (index, pair) in list.iter_mut().enumerate() {
        for name in pair.iter_mut() {
            if let Some(followed) = face(doc, earlier, name, &format!("edges.{index}"))? {
                *name = followed;
                changed = true;
            }
        }
    }
    Ok(changed)
}

/// `feature` with the faces and edges it takes given by the names of the
/// newest body of their chain, at its place in the history (`earlier` are
/// the results before it); `None` when there is nothing to follow, which
/// is every feature that does not take faces of a body. An error is the
/// reason the feature fails.
pub(crate) fn feature(
    doc: &Document,
    earlier: &[FeatureResult],
    feature: &Feature,
) -> Result<Option<Feature>, String> {
    let mut kind = feature.kind.clone();
    let changed = match &mut kind {
        FeatureKind::Chamfer(chamfer) => pairs(doc, earlier, &mut chamfer.edges)?,
        FeatureKind::Fillet(fillet) => pairs(doc, earlier, &mut fillet.edges)?,
        FeatureKind::OffsetFace(offset) => faces(doc, earlier, &mut offset.faces, "faces")?,
        FeatureKind::MoveFace(moving) => faces(doc, earlier, &mut moving.faces, "faces")?,
        FeatureKind::Thicken(layer) => faces(doc, earlier, &mut layer.faces, "faces")?,
        FeatureKind::Draft(tilted) => {
            let mut changed = faces(doc, earlier, &mut tilted.faces, "faces")?;
            if let craft3d_doc::PlaneRef::Face { face: name } = &mut tilted.neutral
                && let Some(followed) = face(doc, earlier, name, "neutral")?
            {
                *name = followed;
                changed = true;
            }
            changed
        }
        FeatureKind::DeleteFace(deletion) => faces(doc, earlier, &mut deletion.faces, "faces")?,
        FeatureKind::Shell(shell) => {
            // The body to hollow is followed as its faces are: the shell
            // takes it.
            let moved = match chain(earlier, shell.source).last() {
                Some(newest) => {
                    shell.source = *newest;
                    true
                }
                None => false,
            };
            faces(doc, earlier, &mut shell.open, "open")? || moved
        }
        FeatureKind::Thread(thread) => {
            let mut changed = false;
            for (name, param) in [(&mut thread.face, "face"), (&mut thread.start, "start")] {
                if let Some(followed) = face(doc, earlier, name, param)? {
                    *name = followed;
                    changed = true;
                }
            }
            changed
        }
        _ => return Ok(None),
    };
    Ok(changed.then(|| Feature {
        kind,
        ..feature.clone()
    }))
}

/// The shortest name that [`face`] follows to `name`, a face of the body
/// of `feature` as it stands at the end of `results`: `name` without as
/// many of the copies in front as can be left out (each step of the
/// chain is then found again), or `name` itself.
pub fn short_face_name(
    doc: &Document,
    results: &[FeatureResult],
    name: &PersistentName,
) -> PersistentName {
    // The names inside, from the innermost (the face as it was made) out.
    let mut inner: Vec<&PersistentName> = Vec::new();
    let mut link = name;
    while link.role == NameRole::Copy {
        let Some(of) = link.of.as_deref() else {
            break;
        };
        if inner.len() >= craft3d_doc::MAX_NAME_DEPTH {
            return name.clone();
        }
        inner.push(of);
        link = of;
    }
    for candidate in inner.into_iter().rev() {
        if let Ok(Some(followed)) = face(doc, results, candidate, "")
            && followed == *name
        {
            return candidate.clone();
        }
    }
    name.clone()
}
