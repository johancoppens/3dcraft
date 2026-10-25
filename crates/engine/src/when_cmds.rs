//! `feature.when`: a feature switched on or off by a condition.
//!
//! The condition is an expression kept with the feature
//! (`craft3d_doc::Feature::when`); regeneration evaluates it every time
//! and builds the feature only when it is a number other than zero
//! (`craft3d_regen`, `pass`). A feature whose condition is zero is
//! *inactive*: it behaves exactly as a suppressed one, passing on the body
//! it changes. A public parameter (a boolean, a choice) then switches
//! features from a form: `when: "lid"`, `when: "profile == 2"`.

use crate::commands::{Run, add_failed, edit, fail, failing, feature_ref, feature_result, spec};
use crate::params::Params;
use crate::{CommandError, CommandSpec, Session};
use craft3d_doc::Expr;
use serde_json::Value;
use std::borrow::Cow;

/// How a command that adds features declares the optional `when`
/// parameter in its params: the condition the features it adds get, as if
/// `feature.when` had been given in the same step. The one code path is in
/// the dispatch ([`take_when`]) and the edit (`commands::edit_merging`), so
/// a command only declares it.
pub(crate) const WHEN_PARAM: &str = r#""when": expr = always on"#;

/// A condition as a parameter gives it: an expression, or `None` for
/// null (no condition, always on).
fn condition(command: &str, key: &str, value: &Value) -> Result<Option<Expr>, CommandError> {
    let source = match value {
        Value::Null => return Ok(None),
        Value::String(text) => text.clone(),
        Value::Number(n) => match n.as_f64().filter(|x| x.is_finite()) {
            Some(x) => x.to_string(),
            None => return fail(format!("{command}: `{key}` must be a finite number")),
        },
        other => {
            return fail(format!(
                "{command}: `{key}` must be a condition: an expression such as \"lid\" or \"profile == 2\", a number, or null; got {other}"
            ));
        }
    };
    if let Err(error) = craft3d_expr::Expr::parse(&source) {
        return fail(format!("{command}: `{key}`: `{source}`: {error}"));
    }
    Ok(Some(Expr::new(source)))
}

/// For a command that declares [`WHEN_PARAM`]: takes `when` out of its
/// parameters (the command itself never sees it) and returns the
/// condition the features it adds get. `None` for every other command,
/// whose parameters stay as they are (an unknown `when` is refused there
/// as any unknown parameter is).
pub(crate) fn take_when(
    spec: &CommandSpec,
    params: &mut Cow<'_, Value>,
) -> Result<Option<Expr>, CommandError> {
    if !spec.params.contains(WHEN_PARAM) {
        return Ok(None);
    }
    let Some(value) = params.get("when") else {
        return Ok(None);
    };
    let when = condition(spec.id, "when", value)?;
    if let Value::Object(map) = params.to_mut() {
        map.remove("when");
    }
    Ok(when)
}

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "feature.when",
    "Switch Feature by Condition",
    r#"{"feature": id or name, "expr": string or number or null}"#,
    "Gives a feature a condition that switches it on and off: it is built when expr evaluates to a plain number other than zero (a comparison such as profile == 2, a boolean public parameter such as lid, && || !, if), and is inactive when it is zero. Inactive behaves exactly as suppressed (feature.suppress): a feature that changes a body passes it on unchanged (an inactive transform leaves the body where it was: put the condition on the feature that makes the body to make it disappear), one that makes a body makes none, a boolean whose tool is inactive passes its target on, a union whose target is inactive passes its tool on, a subtraction or intersection whose target is inactive passes nothing on and still takes its tool, a fillet, chamfer or face operation leaves out the faces and edges of what is inactive and with none left passes its body on, and the inactive feature's appearance and material do not apply. The condition is evaluated at every regeneration, so changing a variable (param.set on a Jig) switches the feature; a condition that does not evaluate, or has a unit, fails the feature with the reason. A suppressed feature stays off whatever its condition. expr null takes the condition away (always on). Renaming a variable rewrites the conditions that use it. A command that adds a feature (the primitives, extrusions and the other features from sketches, booleans, patterns, blends, face operations, threads, datums; not sketch.create, body.split and file.import) takes the condition at once as when: the feature is added with it in the same step and its first regeneration already honours it, so a feature that could not be built while switched off never fails on the way. One undo step. Returns the feature with its status (ok, inactive, ...), its when, what it passes on (passesOn, passesNothing), and under failed the features the change made fail.",
    doc: true,
    feature_when
)];

fn feature_when(session: &mut Session, params: &Value) -> Run {
    const ID: &str = "feature.when";
    let p = Params::new(ID, params, &["feature", "expr"])?;
    let id = feature_ref(session.document()?.history.current(), &p, ID, "feature")?;
    // `null` is a value here (no condition), not a missing parameter.
    let when = match params.get("expr") {
        None => return Err(p.missing("expr")),
        Some(value) => condition(ID, "expr", value)?,
    };
    let name = session
        .document()?
        .history
        .current()
        .feature(id)
        .map(|f| f.name.clone())
        .unwrap_or_default();
    let label = match &when {
        Some(when) => format!("Switch {name} by {}", when.source()),
        None => format!("Switch {name} always on"),
    };
    let before = failing(session)?;
    edit(session, ID, &label, |doc| {
        doc.feature_mut(id)?.when = when;
        Ok(())
    })?;
    let mut result = feature_result(session, id)?;
    add_failed(session, &before, Some(id), &mut result)?;
    Ok(result)
}
