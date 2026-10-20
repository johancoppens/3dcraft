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
//! `body.split` chooses the face for each lump ([`anchors`]): the least
//! name, in the order names compare in, among the faces whose every
//! piece lies in that lump; when the lump has no such face, the least
//! name among its faces with the lump's number.

use crate::derive::source_body_shared;
use crate::entity::sort_in_space;
use crate::{FeatureResult, Names};
use craft3d_doc::{Document, Feature, FeatureId, FeatureKind, PersistentName, Split};
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

/// Whether a split of the body of `source` may read that body although
/// `taker` took it: when `taker` is itself a split of `source`. The
/// splits of one body are made together and share it.
pub(crate) fn shared_by_splits(doc: &Document, source: FeatureId, taker: FeatureId) -> bool {
    matches!(
        doc.feature(taker).map(|f| &f.kind),
        Some(FeatureKind::Split(split)) if split.source == source
    )
}

/// The splits of the body of `source` among `results` that were built,
/// in history order: the group a name of that body is followed through.
pub(crate) fn siblings(
    doc: &Document,
    results: &[FeatureResult],
    source: FeatureId,
) -> Vec<FeatureId> {
    results
        .iter()
        .filter(|r| r.names.is_some() && shared_by_splits(doc, source, r.id))
        .map(|r| r.id)
        .collect()
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
type ByName<'a> = BTreeMap<&'a PersistentName, Vec<usize>>;

fn by_name<'a>(sorted: &Sorted, names: &'a Names) -> ByName<'a> {
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
        .map(|(name, lumps)| (name, lumps.into_iter().collect()))
        .collect()
}

/// The lumps that hold a face answering to `name`.
fn holding<'a>(by_name: &'a ByName<'_>, name: &PersistentName) -> &'a [usize] {
    by_name.get(name).map_or(&[], Vec::as_slice)
}

/// `which` (lumps of `sorted`) in the order of where they lie in space:
/// by the centre of each lump's volume (its outer shell measured alone),
/// coordinates within the tolerance counting as one.
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
    sort_in_space(&mut items, doc.tolerance.linear, 0);
    Ok(items.into_iter().map(|(lump, _)| lump).collect())
}

/// The lump of `body` the split `split` is: the one holding the face, or
/// the numbered one among those that do. The reason starts with the
/// parameter, `face` or `number`.
fn find(
    doc: &Document,
    body: &Body,
    names: &Names,
    by_name: &ByName<'_>,
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
            let ordered = in_space(doc, body, sorted, lumps)?;
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

/// For each lump of `body` (in the kernel's order) the face and number
/// a split of it is stored with: the least name among the faces whose
/// every piece lies in the lump, else the least name among its faces
/// with the lump's number among the lumps that hold that face, in the
/// order of where they lie in space. `names` are what the body's faces
/// are called.
pub fn anchors(
    doc: &Document,
    body: &Body,
    names: &Names,
) -> Result<Vec<(PersistentName, Option<u64>)>, String> {
    let sorted = sorted(doc, body)?;
    anchors_sorted(doc, body, names, &sorted)
}

fn anchors_sorted(
    doc: &Document,
    body: &Body,
    names: &Names,
    sorted: &Sorted,
) -> Result<Vec<(PersistentName, Option<u64>)>, String> {
    let by_name = by_name(sorted, names);
    // The names of each lump's faces, least first, each once.
    let mut own: Vec<BTreeSet<&PersistentName>> = vec![BTreeSet::new(); sorted.lumps.len()];
    for (face, name) in names.faces.iter().enumerate() {
        if let Some(lump) = sorted.lump_of_face.get(face).copied().flatten()
            && let Some(set) = own.get_mut(lump)
        {
            set.insert(name);
        }
    }
    let mut out = Vec::with_capacity(sorted.lumps.len());
    for (lump, own) in own.iter().enumerate() {
        let alone = own.iter().find(|name| holding(&by_name, name) == [lump]);
        if let Some(name) = alone {
            out.push(((*name).clone(), None));
            continue;
        }
        let Some(&name) = own.first() else {
            return Err(format!("lump {lump} has no face to tell it by"));
        };
        let ordered = in_space(doc, body, sorted, holding(&by_name, name))?;
        let number = ordered
            .iter()
            .position(|&l| l == lump)
            .ok_or_else(|| format!("lump {lump} is not among the lumps of its own face"))?;
        out.push((name.clone(), Some(number as u64)));
    }
    Ok(out)
}

/// The body of the split `split` (feature `feature`): the lump of the
/// source's body it names, with the names of its entities as copies.
pub(crate) fn build(
    doc: &Document,
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
    let by_name = by_name(sorted, names);
    let lump = find(doc, body, names, &by_name, sorted, &made_by, split)?;
    // A lump is one split's: when a split of the same body built before
    // this one holds the same lump (the pieces grew together, or the
    // body is one lump now), this one is not the piece it was made for
    // and fails rather than being that lump too. Only the splits before
    // it in the history count, as every feature is a function of the
    // ones before it (the cache lists them among what a split is made
    // from); the first split of a merged lump is that lump, grown, as a
    // face cut in pieces is still the face.
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
