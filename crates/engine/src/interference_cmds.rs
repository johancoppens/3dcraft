//! `document.interference`: which bodies of the document are clear of
//! each other, touch or overlap, without changing the document.
//!
//! Read-only: no feature is added, no undo step is made and the document
//! does not become dirty. Pairs whose boxes (grown by twice the
//! tolerance) do not meet are clear at the cost of two boxes; the others
//! go to the kernel's interference query (`kernel::boolean::interference`),
//! which builds on the boolean: their intersection, and where it is empty,
//! the boolean's section for contact. A pair the boolean refuses is
//! `unknown`, with the reason, never `clear`.

use crate::commands::{Run, fail, feature_ref, length_unit, vec3};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session, commands::spec};
use craft3d_doc::{Document, FeatureId};
use craft3d_kernel::boolean::{Interference, Operand, interference};
use craft3d_math::{Bbox3, Tolerance};
use craft3d_regen::Regen;
use craft3d_topo::{Body, FaceId};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const ID: &str = "document.interference";

/// Most bodies one question looks at.
const MAX_BODIES: usize = 2_000;
/// Most pairs one question may name.
const MAX_PAIRS: usize = 10_000;
/// Most pairs one answer lists; beyond it, clear pairs are only counted.
const MAX_LISTED: usize = 5_000;
/// Most face names listed per body of a pair.
const MAX_FACES: usize = 200;
/// No more threads than this.
#[cfg(not(target_arch = "wasm32"))]
const MAX_THREADS: usize = 8;

pub(crate) static SPECS: &[CommandSpec] = &[
    spec!("document.interference", "Interference", r#"{"bodies": [feature id or name, ...] = all bodies, "pairs": [[a, b], ...], "details": boolean = true}"#,
        "Checks bodies for clashes without changing the document (no feature, no undo step, not dirty). For every pair of the bodies (or only the pairs given; not both), in order, status is clear (no point in common), touch (contact without common volume; contact: face for a shared area, edge for contact along a curve such as an edge on a face or a cylinder lying on a plane, point for isolated points), overlap (volume: the common volume) or unknown (reason: the boolean refused the pair, e.g. contact at the edge of tolerance; never reported as clear). With details: for touch the contact points (at) and the faces of each body the contact is on, for overlap the common volume's bbox and centroid and the faces of each body that bound it, by persistent name. Pairs whose boxes do not meet cost no boolean (work: pairs, and checked: the pairs past that box test, each one boolean, two when faces of both lie on one surface facing each other). Summary counts per status and the total overlap volume; listed pairs are capped at 5000 (clear ones are then only counted).", doc: true, document_interference),
];

/// One body taking part: the feature that made it, its name and box.
struct Part {
    id: FeatureId,
    name: String,
    body: Arc<Body>,
    bbox: Option<Bbox3>,
}

fn document_interference(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(ID, params, &["bodies", "pairs", "details"])?;
    let details = p.bool("details")?.unwrap_or(true);
    if p.get("bodies").is_some() && p.get("pairs").is_some() {
        return fail(format!(
            "{ID}: give `bodies` or `pairs`, not both (pairs names its bodies)"
        ));
    }
    let open = session.document_mut()?;
    let regen = open.regen();
    let doc = open.history.current();
    let tolerance =
        Tolerance::new(doc.tolerance.linear, doc.tolerance.angular).ok_or_else(|| {
            CommandError::new(format!(
                "{ID}: the document's tolerance is not positive and finite"
            ))
        })?;

    // The bodies, and the pairs as indices into them.
    let mut parts: Vec<Part> = Vec::new();
    let index_of = |id: FeatureId, parts: &mut Vec<Part>| -> Result<usize, CommandError> {
        if let Some(at) = parts.iter().position(|part| part.id == id) {
            return Ok(at);
        }
        let part = part_of(doc, &regen, id)?;
        parts.push(part);
        Ok(parts.len() - 1)
    };
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    if let Some(given) = p.get("pairs") {
        let Value::Array(items) = given else {
            return fail(format!(
                "{ID}: `pairs` takes a list of pairs such as [[\"Front\", \"Left\"]]"
            ));
        };
        if items.len() > MAX_PAIRS {
            return fail(format!(
                "{ID}: at most {MAX_PAIRS} pairs at once, got {}",
                items.len()
            ));
        }
        for item in items {
            let Some([a, b]) = item.as_array().map(Vec::as_slice) else {
                return fail(format!(
                    "{ID}: each pair is a list of two bodies (feature id or name), got {item}"
                ));
            };
            let a = index_of(feature(doc, a)?, &mut parts)?;
            let b = index_of(feature(doc, b)?, &mut parts)?;
            if a == b {
                return fail(format!(
                    "{ID}: a pair of `{}` with itself; a pair is two different bodies",
                    parts.get(a).map_or("?", |part| part.name.as_str())
                ));
            }
            pairs.push((a, b));
        }
    } else {
        match p.get("bodies") {
            Some(Value::Array(items)) => {
                if items.len() > MAX_BODIES {
                    return fail(format!(
                        "{ID}: at most {MAX_BODIES} bodies at once, got {}",
                        items.len()
                    ));
                }
                for item in items {
                    index_of(feature(doc, item)?, &mut parts)?;
                }
            }
            Some(_) => {
                return fail(format!(
                    "{ID}: `bodies` takes a list of feature ids or names"
                ));
            }
            None => {
                for (id, _) in regen.bodies() {
                    if parts.len() >= MAX_BODIES {
                        return fail(format!(
                            "{ID}: the document has more than {MAX_BODIES} bodies; name the ones to check with `bodies`"
                        ));
                    }
                    index_of(id, &mut parts)?;
                }
            }
        }
        for a in 0..parts.len() {
            for b in a.saturating_add(1)..parts.len() {
                pairs.push((a, b));
            }
        }
    }

    // The box test, then the kernel for the pairs it leaves.
    let margin = 2.0 * tolerance.linear;
    let candidates: Vec<usize> = pairs
        .iter()
        .enumerate()
        .filter(|(_, (a, b))| match (parts.get(*a), parts.get(*b)) {
            (Some(a), Some(b)) => boxes_meet(a.bbox.as_ref(), b.bbox.as_ref(), margin),
            _ => false,
        })
        .map(|(n, _)| n)
        .collect();
    let found = in_order(&candidates, |n| {
        let (a, b) = pairs.get(n).copied()?;
        let (a, b) = (parts.get(a)?, parts.get(b)?);
        Some(interference(&a.body, &b.body, &tolerance))
    });

    let mut answers: Vec<Option<Result<Interference, String>>> = vec![None; pairs.len()];
    for (n, result) in candidates.iter().zip(found) {
        if let Some(slot) = answers.get_mut(*n) {
            *slot = Some(match result {
                Some(Ok(found)) => Ok(found),
                Some(Err(error)) => Err(error.to_string()),
                None => Err("the query stopped unexpectedly (a bug in 3DCraft)".to_string()),
            });
        }
    }

    let (mut clear, mut touch, mut overlap, mut unknown) = (0_usize, 0_usize, 0_usize, 0_usize);
    let mut overlap_volume = 0.0;
    let list_clear = pairs.len() <= MAX_LISTED;
    let mut listed = Vec::new();
    for (n, &(a, b)) in pairs.iter().enumerate() {
        let (Some(pa), Some(pb)) = (parts.get(a), parts.get(b)) else {
            continue;
        };
        let mut entry = Map::new();
        entry.insert("a".into(), pa.id.to_string().into());
        entry.insert("aName".into(), pa.name.clone().into());
        entry.insert("b".into(), pb.id.to_string().into());
        entry.insert("bName".into(), pb.name.clone().into());
        let is_clear = match answers.get(n).cloned().flatten() {
            None | Some(Ok(Interference::Clear)) => {
                clear = clear.saturating_add(1);
                entry.insert("status".into(), "clear".into());
                true
            }
            Some(Ok(Interference::Touch {
                kind,
                at,
                faces_a,
                faces_b,
            })) => {
                touch = touch.saturating_add(1);
                entry.insert("status".into(), "touch".into());
                entry.insert("contact".into(), kind.as_str().into());
                if details {
                    entry.insert("at".into(), at.into_iter().map(vec3).collect());
                    entry.insert(
                        "faces".into(),
                        json!({
                            "a": face_names(&regen, pa.id, &faces_a),
                            "b": face_names(&regen, pb.id, &faces_b),
                        }),
                    );
                }
                false
            }
            Some(Ok(Interference::Overlap(common))) => {
                match craft3d_topo::mass_properties(&common.body) {
                    Ok(mass) => {
                        overlap = overlap.saturating_add(1);
                        overlap_volume += mass.volume;
                        entry.insert("status".into(), "overlap".into());
                        entry.insert("volume".into(), mass.volume.into());
                        if details {
                            let mut faces: [BTreeSet<FaceId>; 2] = Default::default();
                            for source in common.merged.iter().flatten() {
                                let side = match source.operand {
                                    Operand::A => &mut faces[0],
                                    Operand::B => &mut faces[1],
                                };
                                side.insert(source.face);
                            }
                            let [fa, fb] = faces.map(|set| set.into_iter().collect::<Vec<_>>());
                            if let Some((lo, hi)) =
                                common.body.bbox().ok().and_then(|b| b.corners())
                            {
                                entry.insert(
                                    "bbox".into(),
                                    json!({"min": vec3(lo), "max": vec3(hi)}),
                                );
                            }
                            entry
                                .insert("centroid".into(), mass.centroid.map_or(Value::Null, vec3));
                            entry.insert(
                                "faces".into(),
                                json!({
                                    "a": face_names(&regen, pa.id, &fa),
                                    "b": face_names(&regen, pb.id, &fb),
                                }),
                            );
                        }
                        false
                    }
                    Err(error) => {
                        unknown = unknown.saturating_add(1);
                        entry.insert("status".into(), "unknown".into());
                        entry.insert(
                            "reason".into(),
                            format!("the common volume could not be measured: {error}").into(),
                        );
                        false
                    }
                }
            }
            Some(Err(reason)) => {
                unknown = unknown.saturating_add(1);
                entry.insert("status".into(), "unknown".into());
                entry.insert("reason".into(), reason.into());
                false
            }
        };
        if (list_clear || !is_clear) && listed.len() < MAX_LISTED {
            listed.push(Value::Object(entry));
        }
    }
    let unit = length_unit(doc);
    Ok(json!({
        "bodies": parts.iter().map(|part| json!({"feature": part.id.to_string(), "name": part.name})).collect::<Vec<_>>(),
        "pairs": listed,
        "count": pairs.len(),
        "listed": listed.len(),
        "summary": {
            "clear": clear,
            "touch": touch,
            "overlap": overlap,
            "unknown": unknown,
            "overlapVolume": overlap_volume,
        },
        "work": {"pairs": pairs.len(), "checked": candidates.len()},
        "lengthUnit": unit,
        "volumeUnit": format!("{unit}^3"),
    }))
}

/// The feature a value names (an id such as 3 or "F3", or a name).
fn feature(doc: &Document, value: &Value) -> Result<FeatureId, CommandError> {
    let wrapped = json!({ "body": value });
    let p = Params::new(ID, &wrapped, &["body"])?;
    feature_ref(doc, &p, ID, "body")
}

fn part_of(doc: &Document, regen: &Regen, id: FeatureId) -> Result<Part, CommandError> {
    let name = doc
        .feature(id)
        .map_or_else(|| id.to_string(), |f| f.name.clone());
    let Some(outcome) = regen.feature(id) else {
        return fail(format!("{ID}: there is no feature {id}"));
    };
    let Some(body) = outcome.own_body() else {
        return fail(match outcome.consumed_by {
            Some(taker) => {
                format!("{ID}: the body of `{name}` went into {taker}; name that feature")
            }
            None => format!("{ID}: `{name}` has no body"),
        });
    };
    Ok(Part {
        id,
        name,
        bbox: body.bbox().ok(),
        body: Arc::clone(body),
    })
}

/// The boxes, grown by `margin`, meet. A body without a box is taken to
/// meet everything, so that the kernel decides and reports it.
fn boxes_meet(a: Option<&Bbox3>, b: Option<&Bbox3>, margin: f64) -> bool {
    let (Some((alo, ahi)), Some((blo, bhi))) =
        (a.and_then(Bbox3::corners), b.and_then(Bbox3::corners))
    else {
        return true;
    };
    alo.x <= bhi.x + margin
        && blo.x <= ahi.x + margin
        && alo.y <= bhi.y + margin
        && blo.y <= ahi.y + margin
        && alo.z <= bhi.z + margin
        && blo.z <= ahi.z + margin
}

/// The persistent names of `faces` of the body of `id` (at most
/// [`MAX_FACES`]); a face without one is given by its index.
fn face_names(regen: &Regen, id: FeatureId, faces: &[FaceId]) -> Vec<Value> {
    let names = regen.feature(id).and_then(|f| f.names.as_ref());
    faces
        .iter()
        .take(MAX_FACES)
        .map(|face| {
            names
                .and_then(|n| n.faces.get(face.0 as usize))
                .map_or_else(|| Value::from(face.0), |name| name.to_string().into())
        })
        .collect()
}

/// `job` for every index of `indices`, on several threads; the results in
/// the order of `indices`. `None` for a job whose thread stopped.
fn in_order<T: Send>(indices: &[usize], job: impl Fn(usize) -> Option<T> + Sync) -> Vec<Option<T>> {
    #[cfg(target_arch = "wasm32")]
    let threads = 1;
    #[cfg(not(target_arch = "wasm32"))]
    let threads = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(MAX_THREADS);
    let threads = threads.min(indices.len());
    let one = |n: usize| indices.get(n).and_then(|&index| job(index));
    if threads <= 1 {
        return (0..indices.len()).map(one).collect();
    }
    let next = AtomicUsize::new(0);
    let work = || -> Vec<(usize, Option<T>)> {
        let mut done = Vec::new();
        loop {
            let n = next.fetch_add(1, Ordering::Relaxed);
            if n >= indices.len() {
                return done;
            }
            done.push((n, one(n)));
        }
    };
    let mut slots: Vec<Option<T>> = (0..indices.len()).map(|_| None).collect();
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 1..threads {
            if let Ok(worker) = std::thread::Builder::new().spawn_scoped(scope, work) {
                workers.push(worker);
            }
        }
        let mut done = work();
        for worker in workers {
            // A worker that stopped leaves its pairs `None`: unknown.
            if let Ok(more) = worker.join() {
                done.extend(more);
            }
        }
        for (n, result) in done {
            if let Some(slot) = slots.get_mut(n) {
                *slot = result;
            }
        }
    });
    slots
}
