//! One lump of a body as a body of its own (`body.split`): the shell that
//! bounds the lump from outside and the cavities in it, as
//! `craft3d_kernel::lumps` sorts the shells of a solid.
//!
//! A split feature takes the body of its source, as every feature made
//! from a body does, but the other lumps are split features of their own
//! with the same source, made together, and the source's body is shared
//! by all of them: a body taken by a split is open to every split of it
//! (`shared_by_splits`), and a name of the source is followed through the
//! group as through one taker ([`crate::follow`]).
//!
//! Which lump a split is, is said by a face of the source's body, never
//! by the order of the shells (which the kernel's sewing sets and an
//! upstream edit may change): the lump that holds a piece of the face
//! `face`. Pieces of one face lie in one lump, or in several (a ring cut
//! in two crescents: both have pieces of the wall and of both cut faces);
//! then `number` says which, by where the lumps lie in space, in the one
//! order entities that no face tells apart are numbered in
//! (`entity::sort_in_space`: by x, then y, then z of the centre of each
//! lump's volume, coordinates within the tolerance counting as one). No
//! number means the face lies in one lump only, and the feature fails
//! when an edit puts pieces of it in several; a number fails when the
//! face lies in one lump, or in fewer than it counts to: a name never
//! resolves to another lump (docs/design/names.md).
//!
//! The splits of one body are its lumps, all of them: a lump is one
//! split's (a later split that names a lump an earlier one holds fails),
//! and a lump no split holds (an upstream edit made one more, a split
//! was deleted) fails the last split of the group, so that no material
//! goes missing in silence; `body.split` on the source then adds splits
//! for the lumps that have none. Each split's result depends on the
//! other splits of its body (which they are, whether they are built,
//! which lump each names), so the cache keeps that with the split's
//! context ([`group_state`]).
//!
//! `body.split` chooses the face for each lump ([`anchors`]): the least
//! name, in the order names compare in, among the faces whose every
//! piece lies in that lump; when the lump has no such face, the least
//! name among its faces with the lump's number.

use crate::derive::source_body_shared;
use crate::entity::sort_in_space;
use crate::{FeatureResult, Names};
use craft3d_doc::{Document, Feature, FeatureId, FeatureKind, PersistentName, Split};
use craft3d_expr::Evaluation;
use craft3d_kernel::lumps::{Lump, lumps};
use craft3d_math::{Tolerance, Vec3};
use craft3d_topo::{Body, BodyKind, ShellId, mass_properties, shell_alone, shells_alone};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError, Weak};

/// The lumps of a body, with the lump each face is in.
pub(crate) struct Sorted {
    pub lumps: Vec<Lump>,
    /// Per face of the body, the lump (by index into `lumps`) whose outer
    /// shell or cavities hold it; `None` for a face of no shell, which
    /// `check` refuses.
    lump_of_face: Vec<Option<usize>>,
}

/// Most bodies whose sorting is remembered; the oldest go first.
const MAX_REMEMBERED: usize = 64;

/// The sorting of the bodies split lately, by body: every split of one
/// body reads the same `Arc`, and sorting the shells of a body of many
/// lumps casts rays between every pair of shells that can be nested, so
/// it is done once per body, not once per split. The `Arc`'s address is
/// held by a `Weak`, so it cannot be taken by another body while the
/// entry exists; the answer is the very value a fresh sorting gives.
static REMEMBERED: Mutex<Vec<(Weak<Body>, Remembered)>> = Mutex::new(Vec::new());

/// A body's sorting as remembered: shared by every split that asks.
type Remembered = Arc<Result<Sorted, String>>;

/// The lumps of every name, by body and names ([`by_name`]), remembered
/// as the sorting is.
static REMEMBERED_NAMES: Mutex<Vec<NamesEntry>> = Mutex::new(Vec::new());

/// One remembered [`by_name`]: the body and names it is of, and the map.
type NamesEntry = (Weak<Body>, Weak<Names>, Arc<ByName>);

/// [`sorted`] of `body`, sorted at most once per body. The lock is not
/// held while sorting: two threads that ask at once both sort.
pub(crate) fn sorted_remembered(doc: &Document, body: &Arc<Body>) -> Remembered {
    let key = Arc::downgrade(body);
    {
        let remembered = REMEMBERED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, found)) = remembered.iter().find(|(seen, _)| seen.ptr_eq(&key)) {
            return Arc::clone(found);
        }
    }
    let found = Arc::new(sorted(doc, body));
    let mut remembered = REMEMBERED.lock().unwrap_or_else(PoisonError::into_inner);
    remembered.retain(|(seen, _)| seen.strong_count() > 0);
    if let Some((_, there)) = remembered.iter().find(|(seen, _)| seen.ptr_eq(&key)) {
        return Arc::clone(there);
    }
    if remembered.len() >= MAX_REMEMBERED {
        drop(remembered.remove(0));
    }
    remembered.push((key, Arc::clone(&found)));
    found
}

/// [`by_name`] of `body` (sorted as `sorted`) and `names`, made at most
/// once per body and names.
fn by_name_remembered(sorted: &Sorted, body: &Arc<Body>, names: &Arc<Names>) -> Arc<ByName> {
    let (body_key, names_key) = (Arc::downgrade(body), Arc::downgrade(names));
    let same = |(b, n, _): &NamesEntry| b.ptr_eq(&body_key) && n.ptr_eq(&names_key);
    {
        let remembered = REMEMBERED_NAMES
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((_, _, found)) = remembered.iter().find(|entry| same(entry)) {
            return Arc::clone(found);
        }
    }
    let found = Arc::new(by_name(sorted, names));
    let mut remembered = REMEMBERED_NAMES
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    remembered.retain(|(b, n, _)| b.strong_count() > 0 && n.strong_count() > 0);
    if let Some((_, _, there)) = remembered.iter().find(|entry| same(entry)) {
        return Arc::clone(there);
    }
    if remembered.len() >= MAX_REMEMBERED {
        drop(remembered.remove(0));
    }
    remembered.push((body_key, names_key, Arc::clone(&found)));
    found
}

/// Whether a split of the body of `source` may read that body although
/// `taker` took it: when `taker` is itself a split of `source`. The
/// splits of one body are made together and share it.
pub fn shared_by_splits(doc: &Document, source: FeatureId, taker: FeatureId) -> bool {
    matches!(
        doc.feature(taker).map(|f| &f.kind),
        Some(FeatureKind::Split(split)) if split.source == source
    )
}

/// The splits of the body of `source` among `results` that were built,
/// in history order: the group a name of that body is followed through,
/// and what took the body.
pub fn siblings(doc: &Document, results: &[FeatureResult], source: FeatureId) -> Vec<FeatureId> {
    results
        .iter()
        .filter(|r| r.names.is_some() && shared_by_splits(doc, source, r.id))
        .map(|r| r.id)
        .collect()
}

/// The splits of the body of `source` in the document, in history
/// order, each with whether it is built: switched on (not suppressed,
/// its condition not 0 and not failing) and not after the rollback marker
/// (a feature that is not built names no lump).
fn group<'a>(
    doc: &'a Document,
    variables: &Evaluation,
    source: FeatureId,
) -> Vec<(&'a Feature, bool)> {
    let mut rolled_back = false;
    let mut out = Vec::new();
    for feature in &doc.features {
        if let FeatureKind::Split(split) = &feature.kind
            && split.source == source
        {
            let on = matches!(crate::pass::switch(variables, feature), Ok(None));
            out.push((feature, !rolled_back && on));
        }
        if doc.rollback == Some(feature.id) {
            rolled_back = true;
        }
    }
    out
}

/// What a split's result depends on besides its own definition and its
/// source: the other splits of the same body as they stand (each as
/// stated, and whether it is built), for the cache to compare. Empty
/// for a feature that is no split.
pub(crate) fn group_state(
    doc: &Document,
    variables: &Evaluation,
    feature: &Feature,
) -> Vec<(Feature, bool)> {
    match &feature.kind {
        FeatureKind::Split(split) => group(doc, variables, split.source)
            .into_iter()
            .map(|(f, built)| (f.clone(), built))
            .collect(),
        _ => Vec::new(),
    }
}

fn tolerance(doc: &Document) -> Result<Tolerance, String> {
    Tolerance::new(doc.tolerance.linear, doc.tolerance.angular)
        .ok_or_else(|| "the document's tolerance is not positive and finite".to_string())
}

/// The lumps of `body` as the kernel sorts them, with the lump of every
/// face. A sheet has no lumps; a body whose shells do not nest as lumps
/// and cavities is refused with the kernel's reason.
pub(crate) fn sorted(doc: &Document, body: &Body) -> Result<Sorted, String> {
    if body.kind != BodyKind::Solid {
        return Err(
            "a sheet has no lumps; splitting a sheet into its pieces is not supported yet"
                .to_string(),
        );
    }
    let found = lumps(body, &tolerance(doc)?).map_err(|e| e.to_string())?;
    let mut lump_of_face = vec![None; body.faces.len()];
    for (index, lump) in found.iter().enumerate() {
        for shell in std::iter::once(lump.outer).chain(lump.cavities.iter().copied()) {
            for face in body
                .shells
                .get(shell.0 as usize)
                .map_or(&[][..], |s| &s.faces)
            {
                if let Some(slot) = lump_of_face.get_mut(face.0 as usize) {
                    *slot = Some(index);
                }
            }
        }
    }
    Ok(Sorted {
        lumps: found,
        lump_of_face,
    })
}

/// Per name a face answers to (its own or an alias), the lumps
/// (ascending, each once) that hold such a face: made once per body and
/// names, so that finding the lump of a face costs a lookup, whatever
/// the number of splits that ask.
type ByName = BTreeMap<PersistentName, Vec<usize>>;

fn by_name(sorted: &Sorted, names: &Names) -> ByName {
    let mut map: BTreeMap<&PersistentName, BTreeSet<usize>> = BTreeMap::new();
    for face in 0..names.faces.len() {
        let Some(lump) = sorted.lump_of_face.get(face).copied().flatten() else {
            continue;
        };
        for name in names.names_of(face) {
            map.entry(name).or_default().insert(lump);
        }
    }
    map.into_iter()
        .map(|(name, lumps)| (name.clone(), lumps.into_iter().collect()))
        .collect()
}

/// The lumps that hold a face answering to `name`.
fn holding<'a>(by_name: &'a ByName, name: &PersistentName) -> &'a [usize] {
    by_name.get(name).map_or(&[], Vec::as_slice)
}

/// `which` (lumps of `sorted`) in the order of where they lie in space:
/// by the centre of each lump's volume (its outer shell measured alone),
/// coordinates within the tolerance counting as one. Two lumps whose
/// centres lie in one place (a lump in another's cavity, pieces
/// symmetric about a point) are refused: no number would tell them
/// apart but the order of the shells, which an edit may change.
fn in_space(
    doc: &Document,
    body: &Body,
    sorted: &Sorted,
    which: &[usize],
) -> Result<Vec<usize>, String> {
    let mut items: Vec<(usize, Vec3)> = Vec::with_capacity(which.len());
    for &lump in which {
        let outer = sorted
            .lumps
            .get(lump)
            .map(|l| l.outer)
            .ok_or_else(|| format!("there is no lump {lump}"))?;
        let alone = shell_alone(body, outer)
            .ok_or_else(|| format!("shell {} refers to what the body does not have", outer.0))?;
        let centre = mass_properties(&alone)
            .map_err(|e| format!("lump {lump} cannot be measured: {e:?}"))?
            .centroid
            .ok_or_else(|| format!("lump {lump} encloses no volume to find its centre by"))?;
        items.push((lump, centre));
    }
    let tolerance = doc.tolerance.linear;
    sort_in_space(&mut items, tolerance, 0);
    if let Some(pair) = items.windows(2).find(|pair| match pair {
        [(_, a), (_, b)] => {
            (a.x - b.x).abs() <= tolerance
                && (a.y - b.y).abs() <= tolerance
                && (a.z - b.z).abs() <= tolerance
        }
        _ => false,
    }) {
        return Err(format!(
            "lumps {} and {} have the centres of their volumes within the tolerance of each other, so where they lie tells them apart no more than their faces do; numbering such lumps is not supported",
            pair.first().map_or(0, |p| p.0),
            pair.get(1).map_or(0, |p| p.0)
        ));
    }
    Ok(items.into_iter().map(|(lump, _)| lump).collect())
}

/// The lump of `body` the split `split` is: the one holding the face, or
/// the numbered one among those that do. The reason starts with the
/// parameter, `face` or `number`.
fn find(
    doc: &Document,
    body: &Body,
    names: &Names,
    by_name: &ByName,
    sorted: &Sorted,
    made_by: &str,
    split: &Split,
) -> Result<usize, String> {
    let face = &split.face;
    let lumps = holding(by_name, face);
    let Some(&first) = lumps.first() else {
        return Err(format!(
            "face: `{made_by}` has no face {face}; its faces are: {}",
            crate::datum::listed(&names.faces)
        ));
    };
    match split.number {
        None if lumps.len() == 1 => Ok(first),
        None => Err(format!(
            "face: pieces of the face {face} lie in {} lumps of `{made_by}` now, so it no longer says which lump this is; split `{made_by}` again",
            lumps.len()
        )),
        Some(_) if lumps.len() == 1 => Err(format!(
            "number: the face {face} lies in one lump of `{made_by}` now, so there is nothing to number; split `{made_by}` again"
        )),
        Some(number) => {
            let ordered = in_space(doc, body, sorted, lumps)
                .map_err(|reason| format!("number: the face {face} of `{made_by}`: {reason}"))?;
            usize::try_from(number)
                .ok()
                .and_then(|n| ordered.get(n).copied())
                .ok_or_else(|| {
                    format!(
                        "number: pieces of the face {face} lie in {} lumps of `{made_by}`, numbered from 0; there is no lump {number}",
                        ordered.len()
                    )
                })
        }
    }
}

/// The face and number a split of lump `lump` is stored with: the least
/// name among the faces whose every piece lies in the lump, else the
/// least name among its faces with the lump's number among the lumps
/// that hold that face, in the order of where they lie in space.
fn anchor(
    doc: &Document,
    body: &Body,
    sorted: &Sorted,
    by_name: &ByName,
    own: &BTreeSet<&PersistentName>,
    lump: usize,
) -> Result<(PersistentName, Option<u64>), String> {
    if let Some(name) = own.iter().find(|name| holding(by_name, name) == [lump]) {
        return Ok(((*name).clone(), None));
    }
    let Some(&name) = own.first() else {
        return Err(format!("lump {lump} has no face to tell it by"));
    };
    let ordered = in_space(doc, body, sorted, holding(by_name, name))
        .map_err(|reason| format!("the face {name}: {reason}"))?;
    let number = ordered
        .iter()
        .position(|&l| l == lump)
        .ok_or_else(|| format!("lump {lump} is not among the lumps of its own face"))?;
    Ok((name.clone(), Some(number as u64)))
}

/// The names of each lump's faces, least first, each once.
fn own_names<'a>(sorted: &Sorted, names: &'a Names) -> Vec<BTreeSet<&'a PersistentName>> {
    let mut own: Vec<BTreeSet<&PersistentName>> = vec![BTreeSet::new(); sorted.lumps.len()];
    for (face, name) in names.faces.iter().enumerate() {
        if let Some(lump) = sorted.lump_of_face.get(face).copied().flatten()
            && let Some(set) = own.get_mut(lump)
        {
            set.insert(name);
        }
    }
    own
}

/// For each lump of `body` (in the kernel's order) the face and number
/// a split of it is stored with ([`anchor`]). `names` are what the
/// body's faces are called.
pub fn anchors(
    doc: &Document,
    body: &Body,
    names: &Names,
) -> Result<Vec<(PersistentName, Option<u64>)>, String> {
    let sorted = sorted(doc, body)?;
    let by_name = by_name(&sorted, names);
    own_names(&sorted, names)
        .iter()
        .enumerate()
        .map(|(lump, own)| anchor(doc, body, &sorted, &by_name, own, lump))
        .collect()
}

/// What `body.split` of the body of `source` (its body `body`, its faces
/// called `names`) would add: the splits of that body there are and the
/// lumps none of them names.
pub struct SplitPlan {
    /// How many lumps the body has.
    pub lumps: usize,
    /// The splits of the body in the document that are built, in history
    /// order.
    pub split_by: Vec<FeatureId>,
    /// The face and number for each lump no built split names, in the
    /// kernel's order of the lumps.
    pub unsplit: Vec<(PersistentName, Option<u64>)>,
}

/// [`SplitPlan`] of the body of `source`. A sheet, and a body whose
/// shells are not lumps and cavities, are refused with the reason.
pub fn split_plan(
    doc: &Document,
    source: FeatureId,
    body: &Body,
    names: &Names,
) -> Result<SplitPlan, String> {
    let sorted = sorted(doc, body)?;
    let by_name = by_name(&sorted, names);
    let made_by = doc
        .feature(source)
        .map_or_else(|| source.to_string(), |f| f.name.clone());
    // Which splits are on depends on the variables (their conditions).
    let variables = doc
        .variable_table()
        .map(|table| table.evaluate())
        .unwrap_or_else(|_| craft3d_expr::VariableTable::new().evaluate());
    let members: Vec<&Feature> = group(doc, &variables, source)
        .into_iter()
        .filter_map(|(f, built)| built.then_some(f))
        .collect();
    let claimed: BTreeSet<usize> = members
        .iter()
        .filter_map(|f| match &f.kind {
            FeatureKind::Split(split) => {
                find(doc, body, names, &by_name, &sorted, &made_by, split).ok()
            }
            _ => None,
        })
        .collect();
    let mut unsplit = Vec::new();
    for (lump, own) in own_names(&sorted, names).iter().enumerate() {
        if !claimed.contains(&lump) {
            unsplit.push(anchor(doc, body, &sorted, &by_name, own, lump)?);
        }
    }
    Ok(SplitPlan {
        lumps: sorted.lumps.len(),
        split_by: members.iter().map(|f| f.id).collect(),
        unsplit,
    })
}

/// The body of the split `split` (feature `feature`): the lump of the
/// source's body it names, with the names of its entities as copies.
pub(crate) fn build(
    doc: &Document,
    variables: &Evaluation,
    feature: &Feature,
    split: &Split,
    earlier: &[FeatureResult],
) -> Result<(Body, Option<Names>), String> {
    let outcome = source_body_shared(doc, earlier, split.source, "source", |taker| {
        shared_by_splits(doc, split.source, taker)
    })?;
    let made_by = doc
        .feature(split.source)
        .map_or_else(|| split.source.to_string(), |f| f.name.clone());
    let Some(shared) = outcome.body.as_ref() else {
        return Err(format!("source: `{made_by}` has no body"));
    };
    let body: &Body = shared;
    let Some(names) = outcome.names.as_ref() else {
        return Err(format!(
            "source: `{made_by}` does not name its faces, so no lump of it can be told by one"
        ));
    };
    let sorted = sorted_remembered(doc, shared);
    let sorted = sorted
        .as_ref()
        .as_ref()
        .map_err(|reason| format!("source: {reason}"))?;
    let by_name = by_name_remembered(sorted, shared, names);
    let lump = find(doc, body, names, &by_name, sorted, &made_by, split)?;
    // A lump is one split's: when a split of the same body built before
    // this one holds the same lump (the pieces grew together, or the
    // body is one lump now), this one is not the piece it was made for
    // and fails rather than being that lump too; the first split of a
    // merged lump is that lump, grown, as a face cut in pieces is still
    // the face.
    let claimed = earlier.iter().find(|r| {
        r.names.is_some()
            && matches!(doc.feature(r.id).map(|f| &f.kind), Some(FeatureKind::Split(sibling))
                if sibling.source == split.source
                    && find(doc, body, names, &by_name, sorted, &made_by, sibling) == Ok(lump))
    });
    if let Some(claimed) = claimed {
        let by = doc
            .feature(claimed.id)
            .map_or_else(|| claimed.id.to_string(), |f| f.name.clone());
        return Err(format!(
            "face: the lump of `{made_by}` that holds the face {} is the body of `{by}` already: the pieces are one lump now; split `{made_by}` again",
            split.face
        ));
    }
    // Every lump is some split's: the last split of the group that is
    // built answers for the lumps none names (a cut that makes one lump
    // more, a split deleted), so that no material goes missing in
    // silence; `body.split` on the source adds the splits they lack.
    let members: Vec<&Feature> = group(doc, variables, split.source)
        .into_iter()
        .filter_map(|(f, built)| built.then_some(f))
        .collect();
    if members.last().is_some_and(|last| last.id == feature.id) {
        let named: BTreeSet<usize> = members
            .iter()
            .filter_map(|f| match &f.kind {
                FeatureKind::Split(sibling) => {
                    find(doc, body, names, &by_name, sorted, &made_by, sibling).ok()
                }
                _ => None,
            })
            .collect();
        let own = own_names(sorted, names);
        let unsplit: Vec<usize> = (0..sorted.lumps.len())
            .filter(|lump| !named.contains(lump))
            .collect();
        if let Some(&first) = unsplit.first() {
            let faces: Vec<String> = own
                .get(first)
                .into_iter()
                .flatten()
                .take(4)
                .map(ToString::to_string)
                .collect();
            return Err(format!(
                "face: `{made_by}` has {} lumps and {} of them are split; the lump holding {} is no split's: split `{made_by}` again to add it",
                sorted.lumps.len(),
                sorted.lumps.len().saturating_sub(unsplit.len()),
                faces.join(", ")
            ));
        }
    }
    let shells: Vec<ShellId> = sorted
        .lumps
        .get(lump)
        .map(|l| {
            std::iter::once(l.outer)
                .chain(l.cavities.iter().copied())
                .collect()
        })
        .unwrap_or_default();
    let (part, taken) = shells_alone(body, &shells).ok_or_else(|| {
        format!("source: a shell of `{made_by}` refers to what the body does not have")
    })?;
    let names = Names::picked(feature.id, Arc::clone(names), &taken)?;
    Ok((part, Some(names)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft3d_doc::{LengthUnit, Units};
    use craft3d_math::Frame;

    /// Two lumps whose centres coincide (the same cube twice: a body no
    /// check passes, but `in_space` measures the shells alone) cannot be
    /// numbered: refused, never ordered by the shells' order.
    #[test]
    fn lumps_whose_centres_coincide_are_not_numbered() {
        let doc = Document::new(Units {
            length: LengthUnit::Millimetre,
            ..Units::default()
        });
        let cube = craft3d_kernel::primitives::block(&Frame::WORLD, 2.0, 2.0, 2.0).unwrap();
        let (body, _) = craft3d_topo::merge(&[&cube, &cube]).unwrap();
        assert_eq!(body.shells.len(), 2);
        let sorted = Sorted {
            lumps: vec![
                Lump {
                    outer: ShellId(0),
                    cavities: Vec::new(),
                },
                Lump {
                    outer: ShellId(1),
                    cavities: Vec::new(),
                },
            ],
            lump_of_face: (0..body.faces.len()).map(|f| Some(f / 6)).collect(),
        };
        let reason = in_space(&doc, &body, &sorted, &[0, 1]).unwrap_err();
        assert!(
            reason.contains("lumps 0 and 1 have the centres of their volumes within the tolerance"),
            "{reason}"
        );
        // Apart, they are ordered by where they lie.
        let moved = craft3d_kernel::primitives::block(
            &Frame::WORLD.at(Vec3::new(5.0, 0.0, 0.0)).unwrap(),
            2.0,
            2.0,
            2.0,
        )
        .unwrap();
        let (apart, _) = craft3d_topo::merge(&[&moved, &cube]).unwrap();
        assert_eq!(in_space(&doc, &apart, &sorted, &[0, 1]).unwrap(), [1, 0]);
    }
}
