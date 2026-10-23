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
//!   boolean whose tool is absent passes its target on, one whose target is
//!   absent passes nothing on (and leaves its tool alone), a pattern,
//!   transform, mirror or shell of an absent body passes nothing on;
//! - a sketch, a datum, an offset of profiles and a split keep the rule
//!   they had: what refers to them fails saying they are off.
//!
//! A feature that is off and cannot pass its body on (that body failed,
//! or went into another feature) is not a failure of its own: it holds
//! the reason, and a feature that reads its body fails with it.

use crate::derive::source_body;
use crate::{FeatureResult, Names, Status};
use craft3d_doc::{Document, Expr, Feature, FeatureId, FeatureKind};
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
        FeatureKind::Boolean(b) => Some(pass(doc, feature.id, earlier, b.target, "target")),
        // A copy beside its source, a layer beside its body: new bodies.
        FeatureKind::Transform(t) if t.keep => Some(Through::nothing(why.to_string())),
        FeatureKind::Mirror(m) if m.keep => Some(Through::nothing(why.to_string())),
        FeatureKind::Thicken(_) => Some(Through::nothing(why.to_string())),
        _ => Some(match kind.source() {
            Some((source, _)) => pass(doc, feature.id, earlier, source, role(kind)),
            None => Through::nothing(why.to_string()),
        }),
    }
}

/// For a feature that is on: what it passes on because a body it reads as
/// an operand is absent (made by a feature that is off, or passed on as
/// nothing). `None` when its operands are there (or it reads none as a
/// body parameter: a fillet or a face operation names faces, and fails
/// saying the feature that made them is off).
pub(crate) fn operand_absent(
    doc: &Document,
    feature: &Feature,
    earlier: &[FeatureResult],
) -> Option<Through> {
    match &feature.kind {
        FeatureKind::Boolean(b) => {
            if let Some(why) = absent(earlier, b.target) {
                return Some(Through::nothing(why));
            }
            absent(earlier, b.tool).map(|_| pass(doc, feature.id, earlier, b.target, "target"))
        }
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
/// copies in instance 0, taken from `source`; or why it cannot be.
fn pass(
    doc: &Document,
    feature: FeatureId,
    earlier: &[FeatureResult],
    source: FeatureId,
    role: &str,
) -> Through {
    if let Some(why) = absent(earlier, source) {
        return Through::nothing(why);
    }
    let outcome = match source_body(doc, earlier, source, role) {
        Ok(outcome) => outcome,
        Err(reason) => return Through::broken(reason),
    };
    let names = match &outcome.names {
        Some(names) => match Names::copied(feature, vec![(0, Arc::clone(names))]) {
            Ok(names) => Some(Arc::new(names)),
            Err(reason) => return Through::broken(reason),
        },
        None => None,
    };
    Through {
        passed: Passed::Body(source),
        body: outcome.body.clone(),
        names,
        instances: Some(Arc::new(vec![Transform::IDENTITY])),
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
/// a body on: the word for its state when the body has no face of that
/// name, which is then one the feature would have made (a fillet's face,
/// an instance of a pattern other than the first). Such a reference
/// fails naming the feature that is off; it never lands on another face.
pub(crate) fn lost(
    outcome: &FeatureResult,
    name: &craft3d_doc::PersistentName,
) -> Option<&'static str> {
    if !outcome.is_off() || name.feature != outcome.id {
        return None;
    }
    let found = name.role == craft3d_doc::NameRole::Copy
        && outcome
            .names
            .as_deref()
            .is_some_and(|names| (0..names.faces.len()).any(|f| names.face_answers_to(f, name)));
    (!found).then(|| word(&outcome.status))
}
