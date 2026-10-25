//! Features switched off: suppressed by the user, or inactive because
//! their condition (`when`) is zero.
//!
//! A feature that is off is not built, and the features after it go on as
//! if it were not in the history (docs/design/regeneration.md ›
//! *Switched off*):
//!
//! - one that changes a body (a boolean's target, a fillet, a chamfer, a
//!   face operation, a shell, a thread, a pattern, a transform or a mirror
//!   that does not keep its source) *passes that body on unchanged*: its
//!   result holds the body it would have changed, with every name a copy
//!   in instance 0 (`F7:copy(0)/F1:face(2)`, as when it is built), and it
//!   takes that body, so that what comes after finds it where it finds the
//!   feature's body when it is on. What the feature would have made (a
//!   fillet's faces, a pattern's other instances) is absent: a reference to
//!   it fails naming the feature that is off, never lands on another face;
//! - one that makes a body of its own (a primitive, an extrusion, a sweep,
//!   a thicken, a transform that keeps its source) passes nothing on, and a
//!   feature that reads its body as an operand goes on without it: a
//!   boolean whose tool is absent passes its target on, a union whose
//!   target is absent passes its tool on, a subtraction or intersection
//!   whose target is absent passes nothing on and takes its tool all the
//!   same ([`consumed`]: from nothing is nothing, and a cutter is never
//!   left over), a pattern, transform, mirror or shell of an absent body
//!   passes nothing on;
//! - a face or an edge that is absent (of a body that is absent, or one
//!   only a feature that is off makes) is *nothing* to a feature that
//!   names faces or edges to change (a fillet, a chamfer, a face
//!   operation, the open faces of a shell, a thread): the feature is built
//!   with the others, and with none left it passes on the body it would
//!   have changed ([`stand_in`]), or nothing when there is none;
//! - a sketch, a datum, an offset of profiles and a split keep the rule
//!   they had: what refers to them fails saying they are off.
//!
//! A feature that is off and cannot pass its body on (that body failed,
//! or went into another feature) is not a failure of its own: it holds
//! the reason, and a feature that reads its body fails with it.

use crate::derive::source_body;
use crate::{FeatureResult, Names, Status};
use craft3d_doc::{BooleanOp, Document, Expr, Feature, FeatureId, FeatureKind, PersistentName};
use craft3d_expr::Evaluation;
use craft3d_math::Transform;
use craft3d_topo::Body;
use std::sync::Arc;

/// What a feature passes on to the features after it, when it is off or
/// an operand of it is absent ([`FeatureResult::passed`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Passed {
    /// Its body is the body of this feature, unchanged (named as copies):
    /// the feature is identity on it, and takes it.
    Body(FeatureId),
    /// No body, and that is how it should be: the feature makes a body of
    /// its own, or the body it changes is absent itself. Says why ("`Lid`
    /// is suppressed"), from the feature that is off.
    Nothing(String),
    /// No body, because the body it would pass on cannot be had (it
    /// failed, went into another feature): what reads it fails with this.
    Broken(String),
}

/// Whether `feature` is on: `Ok(None)` when it is, `Ok(Some(status))`
/// when it is off (`Suppressed` or `Inactive`), `Err` when its condition
/// cannot be evaluated or is not a plain number. Suppression wins: a
/// suppressed feature's condition is not evaluated.
pub(crate) fn switch(variables: &Evaluation, feature: &Feature) -> Result<Option<Status>, String> {
    if feature.suppressed {
        return Ok(Some(Status::Suppressed));
    }
    let Some(when) = &feature.when else {
        return Ok(None);
    };
    Ok((!active(variables, when)?).then_some(Status::Inactive))
}

/// The value of a condition: true when it is a number other than zero.
pub(crate) fn active(variables: &Evaluation, when: &Expr) -> Result<bool, String> {
    let value = crate::number(variables, "when", when)?;
    if value.is_nan() {
        return Err(format!("when: `{}` is not a number", when.source()));
    }
    Ok(value != 0.0)
}

/// Why a feature that is off has no body of its own, in the words a
/// reader of it quotes.
pub(crate) fn off_reason(feature: &Feature, status: &Status) -> String {
    let name = &feature.name;
    match (status, &feature.when) {
        (Status::Inactive, Some(when)) => {
            format!(
                "`{name}` is inactive (its condition `{}` is 0)",
                when.source()
            )
        }
        (Status::Inactive, None) => format!("`{name}` is inactive"),
        _ => format!("`{name}` is suppressed"),
    }
}

/// What a feature passes on in place of its own result.
pub(crate) struct Through {
    pub passed: Passed,
    pub body: Option<Arc<Body>>,
    pub names: Option<Arc<Names>>,
    pub instances: Option<Arc<Vec<Transform>>>,
}

impl Through {
    fn nothing(why: String) -> Self {
        Self {
            passed: Passed::Nothing(why),
            body: None,
            names: None,
            instances: None,
        }
    }

    fn broken(why: String) -> Self {
        Self {
            passed: Passed::Broken(why),
            body: None,
            names: None,
            instances: None,
        }
    }
}

/// The parameter a kind reads the body it changes as, in the reasons.
fn role(kind: &FeatureKind) -> &'static str {
    match kind {
        FeatureKind::Boolean(_) => "target",
        FeatureKind::Chamfer(_) | FeatureKind::Fillet(_) => "edges",
        FeatureKind::OffsetFace(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Draft(_)
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Thicken(_) => "faces",
        _ => "source",
    }
}

/// What the feature `feature` (as built: its names followed), which is
/// off for `why`, passes on. `None` for the kinds that keep the old rule
/// (a sketch, a datum, an offset of profiles, a split: no body, and what
/// refers to them fails).
pub(crate) fn switched_off(
    doc: &Document,
    feature: &Feature,
    why: &str,
    earlier: &[FeatureResult],
) -> Option<Through> {
    let kind = &feature.kind;
    match kind {
        FeatureKind::Sketch(_)
        | FeatureKind::DatumPlane(_)
        | FeatureKind::DatumAxis(_)
        | FeatureKind::DatumPoint(_)
        | FeatureKind::DatumHelix(_)
        | FeatureKind::OffsetProfile(_)
        | FeatureKind::Split(_) => None,
        FeatureKind::Boolean(b) => Some(pass(doc, feature.id, earlier, b.target, "target", 0)),
        // A copy beside its source, a layer beside its body: new bodies.
        FeatureKind::Transform(t) if t.keep => Some(Through::nothing(why.to_string())),
        FeatureKind::Mirror(m) if m.keep => Some(Through::nothing(why.to_string())),
        FeatureKind::Thicken(_) => Some(Through::nothing(why.to_string())),
        _ => Some(match nothing_left(doc, earlier, feature) {
            Some(absent) => pass_stand_in(doc, feature, earlier, absent),
            None => match kind.source() {
                Some((source, _)) => pass(doc, feature.id, earlier, source, role(kind), 0),
                None => Through::nothing(why.to_string()),
            },
        }),
    }
}

/// What a feature that names faces or edges passes on when none of them
/// is left ([`nothing_left`]; `absent` says why): the body it would have
/// changed ([`stand_in`]), or nothing when there is none.
fn pass_stand_in(
    doc: &Document,
    feature: &Feature,
    earlier: &[FeatureResult],
    absent: String,
) -> Through {
    match stand_in(doc, earlier, feature.id) {
        Ok(Some(body)) => pass(doc, feature.id, earlier, body, role(&feature.kind), 0),
        Ok(None) => Through::nothing(absent),
        Err(reason) => Through::broken(reason),
    }
}

/// For a feature that is on: what it passes on because a body it reads as
/// an operand is absent (made by a feature that is off, or passed on as
/// nothing), or because every face or edge it names is absent
/// ([`nothing_left`]). `None` when what it reads is there.
pub(crate) fn operand_absent(
    doc: &Document,
    feature: &Feature,
    earlier: &[FeatureResult],
) -> Option<Through> {
    match &feature.kind {
        FeatureKind::Boolean(b) => {
            if let Some(why) = absent(earlier, b.target) {
                // A union of nothing and the tool is the tool, named as
                // when the union is built (instance 1). A subtraction or
                // an intersection from nothing is nothing, and takes its
                // tool as it would (`consumed`); a tool the union keeps
                // stays a body of its own.
                if b.op == BooleanOp::Union && !b.keep_tool && absent(earlier, b.tool).is_none() {
                    return Some(pass(doc, feature.id, earlier, b.tool, "tool", 1));
                }
                return Some(Through::nothing(why));
            }
            absent(earlier, b.tool).map(|_| pass(doc, feature.id, earlier, b.target, "target", 0))
        }
        FeatureKind::Chamfer(_)
        | FeatureKind::Fillet(_)
        | FeatureKind::OffsetFace(_)
        | FeatureKind::MoveFace(_)
        | FeatureKind::Draft(_)
        | FeatureKind::DeleteFace(_)
        | FeatureKind::Thread(_) => {
            let why = nothing_left(doc, earlier, feature)?;
            Some(pass_stand_in(doc, feature, earlier, why))
        }
        // A layer on no faces is no body.
        FeatureKind::Thicken(_) => nothing_left(doc, earlier, feature).map(Through::nothing),
        FeatureKind::Transform(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Shell(_) => {
            let (source, _) = feature.kind.source()?;
            absent(earlier, source).map(Through::nothing)
        }
        _ => None,
    }
}

/// Why the body of `id` is absent, when it is absent as it should be
/// ([`Passed::Nothing`]): the feature is off and makes a body of its own,
/// or passes nothing on.
pub(crate) fn absent(earlier: &[FeatureResult], id: FeatureId) -> Option<String> {
    match &earlier.iter().find(|r| r.id == id)?.passed {
        Some(Passed::Nothing(why)) => Some(why.clone()),
        _ => None,
    }
}

/// The body of `source` passed on by `feature`: the same body, named as
/// copies in `instance` (0, or 1 for a union's tool, as when the union is
/// built), taken from `source`; or why it cannot be.
fn pass(
    doc: &Document,
    feature: FeatureId,
    earlier: &[FeatureResult],
    source: FeatureId,
    role: &str,
    instance: u64,
) -> Through {
    if let Some(why) = absent(earlier, source) {
        return Through::nothing(why);
    }
    let outcome = match source_body(doc, earlier, source, role) {
        Ok(outcome) => outcome,
        Err(reason) => return Through::broken(reason),
    };
    let names = match &outcome.names {
        Some(names) => match Names::copied(feature, vec![(instance, Arc::clone(names))]) {
            Ok(names) => Some(Arc::new(names)),
            Err(reason) => return Through::broken(reason),
        },
        None => None,
    };
    Through {
        passed: Passed::Body(source),
        body: outcome.body.clone(),
        names,
        // Neither moves: one map per instance up to the one it is.
        instances: Some(Arc::new(if instance == 0 {
            vec![Transform::IDENTITY]
        } else {
            vec![Transform::IDENTITY; 2]
        })),
    }
}

/// "suppressed" or "inactive", for a status that is off.
pub(crate) fn word(status: &Status) -> &'static str {
    match status {
        Status::Inactive => "inactive",
        _ => "suppressed",
    }
}

/// For a name looked up in the result of a feature that is off and passes
/// a body on: the word for its state when the body has no face, edge or
/// vertex of that name (each looked for as its kind is), which is then one the feature would have made (a fillet's face,
/// an instance of a pattern other than the first). Such a reference
/// fails naming the feature that is off; it never lands on another face.
pub(crate) fn lost(
    outcome: &FeatureResult,
    name: &craft3d_doc::PersistentName,
) -> Option<&'static str> {
    if !outcome.is_off() || name.feature != outcome.id {
        return None;
    }
    let found = outcome.names.as_deref().is_some_and(|names| {
        if name.is_vertex() || name.is_edge() {
            // Names of edges and vertices let go of with an evicted body
            // cannot be told: left to the lookup that reads the body.
            !names.has_entities()
                || if name.is_vertex() {
                    names.vertex_named(name).is_some()
                } else {
                    names.edge_named(name).is_some()
                }
        } else {
            name.role == craft3d_doc::NameRole::Copy
                && (0..names.faces.len()).any(|f| names.face_answers_to(f, name))
        }
    });
    (!found).then(|| word(&outcome.status))
}

/// Why the face or edge `name` is absent at this point in the history,
/// when it is: the feature of one of its links (`F7:copy(0)/F2:face(1)`
/// has the links F7 and F2) passes nothing on (it is off and makes a body,
/// or the body it changes is absent), or is off and does not have it
/// ([`lost`]: a face only that feature makes). `None` when it is there,
/// and when what is wrong with it is something else (a feature that
/// failed, comes after, or was deleted), which reading it says.
pub(crate) fn absent_name(
    doc: &Document,
    earlier: &[FeatureResult],
    name: &PersistentName,
) -> Option<String> {
    let mut link = Some(name);
    // A name is finite; the cap is for one built by hand.
    for _ in 0..craft3d_doc::MAX_NAME_DEPTH {
        let here = link?;
        if let Some(outcome) = earlier.iter().find(|r| r.id == here.feature) {
            if let Some(Passed::Nothing(why)) = &outcome.passed {
                return Some(why.clone());
            }
            if let Some(Passed::Body(_)) = &outcome.passed
                && let Some(off) = lost(outcome, here)
            {
                let by = doc
                    .feature(here.feature)
                    .map_or_else(|| here.feature.to_string(), |f| f.name.clone());
                return Some(format!("`{by}`, which makes {here}, is {off}"));
            }
        }
        link = here.of.as_deref();
    }
    None
}

/// For a feature that names faces or edges to change (`feature` as built,
/// with its names of absent faces and edges left out by
/// `crate::follow::feature`): why none of them is left, when every one it
/// was given is absent (for a draft also its neutral face, for a thread
/// its face or its start). `None` when some are left, and for a feature
/// given none, which fails as it always did.
pub(crate) fn nothing_left(
    doc: &Document,
    earlier: &[FeatureResult],
    feature: &Feature,
) -> Option<String> {
    let stated = doc.feature(feature.id).unwrap_or(feature);
    let pairs = |built: &[[PersistentName; 2]], given: &[[PersistentName; 2]]| {
        (built.is_empty() && !given.is_empty())
            .then(|| {
                given
                    .iter()
                    .flatten()
                    .find_map(|name| absent_name(doc, earlier, name))
            })
            .flatten()
    };
    let faces = |built: &[PersistentName], given: &[PersistentName]| {
        (built.is_empty() && !given.is_empty())
            .then(|| {
                given
                    .iter()
                    .find_map(|name| absent_name(doc, earlier, name))
            })
            .flatten()
    };
    match (&feature.kind, &stated.kind) {
        (FeatureKind::Chamfer(built), FeatureKind::Chamfer(given)) => {
            pairs(&built.edges, &given.edges)
        }
        (FeatureKind::Fillet(built), FeatureKind::Fillet(given)) => {
            pairs(&built.edges, &given.edges)
        }
        (FeatureKind::OffsetFace(built), FeatureKind::OffsetFace(given)) => {
            faces(&built.faces, &given.faces)
        }
        (FeatureKind::MoveFace(built), FeatureKind::MoveFace(given)) => {
            faces(&built.faces, &given.faces)
        }
        (FeatureKind::DeleteFace(built), FeatureKind::DeleteFace(given)) => {
            faces(&built.faces, &given.faces)
        }
        (FeatureKind::Thicken(built), FeatureKind::Thicken(given)) => {
            faces(&built.faces, &given.faces)
        }
        (FeatureKind::Draft(built), FeatureKind::Draft(given)) => faces(&built.faces, &given.faces)
            .or_else(|| match &built.neutral {
                // Tilted about a face that is not there: nothing to do.
                craft3d_doc::PlaneRef::Face { face } => absent_name(doc, earlier, face),
                _ => None,
            }),
        (FeatureKind::Thread(built), _) => absent_name(doc, earlier, &built.face)
            .or_else(|| absent_name(doc, earlier, &built.start)),
        _ => None,
    }
}

/// The feature whose body the feature `id`, which names faces or edges to
/// change, would have changed when every one it names is absent
/// ([`nothing_left`]): the body its first name is of, followed to the
/// newest body before it. When that body is absent itself, the body it
/// would have gone into stands in for it: the first later feature that
/// takes it (a union whose tool it is, a fillet of its edges), and so on.
/// `Ok(None)` when none of them has a body (a fillet of the edges of a
/// block that is off, with nothing between); `Err` when one on the way
/// failed, with the reason its body cannot be read. The cache counts the
/// result as one of the feature's inputs (`cache::features`).
pub(crate) fn stand_in(
    doc: &Document,
    earlier: &[FeatureResult],
    id: FeatureId,
) -> Result<Option<FeatureId>, String> {
    let Some(stated) = doc.feature(id) else {
        return Ok(None);
    };
    let first = match &stated.kind {
        FeatureKind::Chamfer(c) => c.edges.first().map(|[a, _]| a),
        FeatureKind::Fillet(f) => f.edges.first().map(|[a, _]| a),
        FeatureKind::OffsetFace(o) => o.faces.first(),
        FeatureKind::MoveFace(m) => m.faces.first(),
        FeatureKind::DeleteFace(d) => d.faces.first(),
        FeatureKind::Thicken(t) => t.faces.first(),
        FeatureKind::Draft(d) => d.faces.first(),
        FeatureKind::Thread(t) => Some(&t.face),
        _ => None,
    };
    let Some(first) = first else {
        return Ok(None);
    };
    let mut at = first.feature;
    // Each step goes to a later feature, so the walk ends; bounded all the
    // same.
    for _ in 0..=earlier.len() {
        let Some(place) = earlier.iter().position(|r| r.id == at) else {
            return Ok(None);
        };
        let Some(outcome) = earlier.get(place) else {
            return Ok(None);
        };
        if let Some(Passed::Nothing(_)) = &outcome.passed {
            // The first feature after it that would take its body.
            let taker = earlier.iter().skip(place.saturating_add(1)).find(|r| {
                doc.feature(r.id).is_some_and(|f| {
                    f.kind
                        .sources()
                        .iter()
                        .any(|&(source, keep)| source == at && !keep)
                })
            });
            match taker {
                Some(taker) => at = taker.id,
                None => return Ok(None),
            }
        } else if outcome.is_built() {
            return Ok(Some(newest_before(earlier, at)));
        } else {
            return source_body(doc, earlier, at, role(&stated.kind)).map(|_| Some(at));
        }
    }
    Ok(None)
}

/// The feature whose body the body of `id` is at the end of `earlier`:
/// followed through the features that took it, as long as they are among
/// `earlier` (a body derived again is read before what took it later).
fn newest_before(earlier: &[FeatureResult], id: FeatureId) -> FeatureId {
    let mut at = id;
    // Each taker comes later; bounded all the same.
    for _ in 0..earlier.len() {
        let taker = earlier
            .iter()
            .find(|r| r.id == at)
            .and_then(|r| r.consumed_by)
            .filter(|taker| *taker != id && earlier.iter().any(|r| r.id == *taker));
        match taker {
            Some(taker) => at = taker,
            None => break,
        }
    }
    at
}

/// The tool a boolean that is on takes although it passes nothing on: a
/// subtraction or an intersection whose target is absent is nothing, and
/// its tool goes into it as it would with the target there (a cutter is
/// never left over as a body of its own). Not with `keepTool`, not for a
/// union (which passes its tool on, and takes it so), and only a tool
/// that is there to take.
pub(crate) fn consumed(
    feature: &Feature,
    result: &FeatureResult,
    earlier: &[FeatureResult],
) -> Option<FeatureId> {
    let FeatureKind::Boolean(b) = &feature.kind else {
        return None;
    };
    if result.status != Status::Ok
        || !matches!(result.passed, Some(Passed::Nothing(_)))
        || b.op == BooleanOp::Union
        || b.keep_tool
    {
        return None;
    }
    absent(earlier, b.target)?;
    let tool = earlier.iter().find(|r| r.id == b.tool)?;
    (tool.is_built() && tool.consumed_by.is_none()).then_some(b.tool)
}
