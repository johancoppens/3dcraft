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
//! (`F11:copy(0)/F2:face(0)`: a run of copies that are each instance 0 is
//! one link, see [`steps`]), and those are what it means: a short name is
//! an abbreviation of exactly one full one.
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
/// `k` for which a face of `names` answers to `taker:copy(k)/name` (in
/// the short form, where a copy of a copy is one link).
fn copies(names: &Names, taker: FeatureId, name: &PersistentName) -> BTreeSet<u64> {
    names
        .all_face_names()
        .filter(|face| face.feature == taker && face.role == NameRole::Copy)
        .filter_map(|face| {
            let k = *face.sources.first()?;
            same_plane(face, &PersistentName::copy(taker, k, name.clone())).then_some(k)
        })
        .collect()
}

/// The feature whose body instance `instance` of `feature` was made from:
/// for a boolean the target (0) or the tool (1), for every other feature
/// made from a body that body, whatever the instance. As the feature was
/// built: names it was given of an earlier body were followed to the
/// newest one. `None` for a feature not made from a body.
pub(crate) fn source_of(
    doc: &Document,
    earlier: &[FeatureResult],
    feature: FeatureId,
    instance: u64,
) -> Option<FeatureId> {
    let built = earlier
        .iter()
        .find(|r| r.id == feature)
        .and_then(|r| r.followed.as_deref());
    let kind = match built {
        Some(built) => &built.kind,
        None => &doc.feature(feature)?.kind,
    };
    let sources = kind.sources();
    let from = match kind {
        FeatureKind::Boolean(_) => sources.get(usize::try_from(instance).ok()?),
        _ => sources.first(),
    };
    from.map(|(id, _)| *id)
}

/// The way of a copy through the history: for
/// `F9:copy(0)/F4:copy(1)/F3:side(e1)` each feature it passes, with the
/// instance the face is in there, from the outside in (`[(F9, 0), (F7, 0),
/// (F5, 0), (F4, 1)]` when F9, F7 and F5 each took the body before as
/// their target and F4 took F3's as its tool), and the name it ends at
/// (`F3:side(e1)`, which is no copy). A link with the one number 0 goes on
/// through instance 0 until it reaches the feature of the link after it;
/// a link with other numbers takes them one step each, the last onto
/// that feature (docs/design/names.md). `None` for a name that is
/// not a copy, and for a way that does not lead there in the history as
/// it is.
pub(crate) fn steps<'a>(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &'a PersistentName,
) -> Option<(Vec<(FeatureId, u64)>, &'a PersistentName)> {
    if name.role != NameRole::Copy {
        return None;
    }
    let mut steps = Vec::new();
    let mut link = name;
    // Each body is made from an earlier one, so the way is no longer than
    // the history; bounded all the same, for a bad file.
    let mut budget = earlier.len().saturating_add(1);
    while link.role == NameRole::Copy {
        let next = link.of.as_deref()?;
        let mut at = link.feature;
        if link.sources == [0] {
            loop {
                budget = budget.checked_sub(1)?;
                steps.push((at, 0));
                at = source_of(doc, earlier, at, 0)?;
                if at == next.feature {
                    break;
                }
            }
        } else {
            for &instance in &link.sources {
                budget = budget.checked_sub(1)?;
                steps.push((at, instance));
                at = source_of(doc, earlier, at, instance)?;
            }
            if at != next.feature {
                return None;
            }
        }
        link = next;
    }
    Some((steps, link))
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
    // Each step goes to the feature that took the body the name is of
    // now, which comes later in the history, so the way is no longer
    // than the history (the chain of one taker's consumption says
    // nothing of the way through a split that is not the last).
    for _ in 0..earlier.len() {
        let Some(taker) = earlier
            .iter()
            .find(|r| r.id == current.feature)
            .and_then(|r| r.consumed_by)
        else {
            break;
        };
        // The splits of one body took it together: the face is looked
        // for in every one of them, as in the instances of one taker.
        let group = if crate::split::shared_by_splits(doc, current.feature, taker) {
            crate::split::siblings(doc, earlier, current.feature)
        } else {
            vec![taker]
        };
        let by = called(doc, taker);
        let kind = doc.feature(taker).map_or("feature", |f| f.kind.type_name());
        let mut found: Vec<(FeatureId, u64)> = Vec::new();
        let mut there: Vec<PersistentName> = Vec::new();
        for &member in &group {
            let Some(names) = earlier
                .iter()
                .find(|r| r.id == member)
                .and_then(|r| r.names.as_ref())
            else {
                return Err(format!(
                    "{role}: the body of `{from}` went into `{by}`, a {kind} that does not name its faces, so {name} cannot be followed there"
                ));
            };
            found.extend(
                copies(names, member, &current)
                    .into_iter()
                    .map(|k| (member, k)),
            );
            there.extend(names.faces.iter().cloned());
        }
        let group_names = || {
            if group.len() > 1 {
                let all: Vec<String> = group.iter().map(|&m| called(doc, m)).collect();
                format!("the splits {}", all.join(", "))
            } else {
                format!("`{by}`")
            }
        };
        match found.as_slice() {
            [] => {
                return Err(format!(
                    "{role}: the face {name} is gone: the body of `{from}` went into {}, a {kind}, which left no face of it; its faces are: {}",
                    group_names(),
                    listed(&there)
                ));
            }
            [(member, k)] => {
                from = called(doc, *member);
                current = PersistentName::copy(*member, *k, current);
            }
            _ => {
                let all: Vec<String> = found
                    .iter()
                    .map(|&(member, k)| {
                        PersistentName::copy(member, k, current.clone()).to_string()
                    })
                    .collect();
                return Err(format!(
                    "{role}: the face {name} is more than one face after {}, a {kind} the body of `{from}` went into: {}; give one of these names",
                    group_names(),
                    all.join(", ")
                ));
            }
        }
    }
    Ok(Some(current))
}

/// [`face`] for a reader of the finished regeneration (an export, a
/// query): the full name, in the newest body of its chain at the end of
/// `results`, of the face `name` of an earlier body, by exactly the rule
/// features follow names by. `None` when that body is its own still;
/// an error, starting with `role`, says why the name cannot be followed
/// (gone, or several copies, which it lists).
pub fn follow_face(
    doc: &Document,
    results: &[FeatureResult],
    name: &PersistentName,
    role: &str,
) -> Result<Option<PersistentName>, String> {
    face(doc, results, name, role)
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
    // The names inside, from the innermost (the face as it was made) out:
    // what the face was called in each body on the way of the copy.
    let Some((steps, end)) = steps(doc, results, name) else {
        return name.clone();
    };
    let mut inner: Vec<PersistentName> = vec![end.clone()];
    for &(feature, instance) in steps.iter().skip(1).rev() {
        let next = PersistentName::copy(feature, instance, inner.last().unwrap_or(end).clone());
        inner.push(next);
    }
    for candidate in inner {
        if let Ok(Some(followed)) = face(doc, results, &candidate, "")
            && followed == *name
        {
            return candidate;
        }
    }
    name.clone()
}
