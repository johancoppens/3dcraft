//! `document.setTolerance`: changes the modelling tolerances of the active
//! document after `file.new` (docs/design/document.md › *Tolerances*).
//!
//! The linear and the angular tolerance are the document's data (AGENTS.md
//! rule 3): what counts as the same place and the same direction, and what
//! the proven measures hold a seam to (`measure.continuity`'s thresholds
//! are the tolerance, the angular tolerance and, for curvature,
//! `min(angular / L, 2 tolerance / L²)`). A command script sets them in
//! its first step, so an example that needs other tolerances than the
//! default carries them (the class-A tank, 2026-10-31). The document is
//! regenerated with them, as after any edit; nothing else changes.

use crate::commands::{Run, edit, fail, spec};
use crate::params::Params;
use crate::{CommandSpec, Session};
use craft3d_doc::Tolerance;
use serde_json::{Value, json};

/// The least angular tolerance, in radians: below it two directions would
/// be told apart by rounding (`docs/decisions.md`: tangency is never held
/// to less than 1e-9 anyway; this leaves room under the default).
pub const LEAST_ANGULAR: f64 = 1e-12;
/// The greatest angular tolerance, in radians (about 0.06°): coarser, two
/// faces visibly creased would count as tangent.
pub const GREATEST_ANGULAR: f64 = 1e-3;

pub(crate) static SPECS: &[CommandSpec] = &[spec!(
    "document.setTolerance",
    "Set Tolerances",
    r#"{"linear": number = kept, "angular": number = kept, "loosest": number or null = kept}"#,
    "Changes the modelling tolerances of the active document (one undo step), as file.new sets them: linear in the document's length unit (1e-12 to 1e3), angular in radians (1e-12 to 1e-3; the default 1e-9), loosest the cap on an entity's own tolerance (at least linear; null takes it away, so it is 1000 times linear). What is not given is kept. The document is regenerated with them: what counts as the same place and direction, and the thresholds the proven measures hold to (measure.continuity: the tolerance, the angular tolerance, and for curvature min(angular / L, 2 linear / L^2) over an edge of length L), so a long seam's G2 is proven against a curvature threshold that a design of that size can state. Returns {linear, angular, loosest, was: the tolerances before}.",
    doc: true,
    set_tolerance
)];

const ID: &str = "document.setTolerance";

fn set_tolerance(session: &mut Session, params: &Value) -> Run {
    let p = Params::new(ID, params, &["linear", "angular", "loosest"])?;
    let current = session.document()?.history.current().tolerance;
    let mut next = current;
    if let Some(linear) = p.number("linear")? {
        if !Tolerance::linear_in_range(linear) {
            return fail(format!(
                "{ID}: `linear` must be between {:e} and {:e} of the document's length unit; got {linear:e}",
                Tolerance::LEAST_LINEAR,
                Tolerance::GREATEST_LINEAR
            ));
        }
        next.linear = linear;
    }
    if let Some(angular) = p.number("angular")? {
        if !(LEAST_ANGULAR..=GREATEST_ANGULAR).contains(&angular) {
            return fail(format!(
                "{ID}: `angular` must be between {LEAST_ANGULAR:e} and {GREATEST_ANGULAR:e} rad; got {angular:e}"
            ));
        }
        next.angular = angular;
    }
    match params.get("loosest") {
        None => {}
        Some(Value::Null) => next.loosest = None,
        Some(_) => {
            let loosest = p.number("loosest")?.unwrap_or(f64::NAN);
            if !(loosest.is_finite() && loosest >= next.linear) {
                return fail(format!(
                    "{ID}: `loosest` must be a finite length at least the linear tolerance {:e}; got {loosest:e}",
                    next.linear
                ));
            }
            next.loosest = Some(loosest);
        }
    }
    if next.loosest.is_some_and(|l| l < next.linear) {
        return fail(format!(
            "{ID}: the cap `loosest` {:e} would be below the linear tolerance {:e}; give a larger loosest or null",
            next.loosest.unwrap_or(0.0),
            next.linear
        ));
    }
    let report = |t: &Tolerance| json!({"linear": t.linear, "angular": t.angular, "loosest": t.loosest_linear()});
    if next != current {
        edit(session, ID, "Set tolerances", |doc| {
            doc.tolerance = next;
            Ok(())
        })?;
    }
    let mut out = report(&next);
    if let Some(map) = out.as_object_mut() {
        map.insert("was".into(), report(&current));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::Session;
    use crate::workspace::Workspace;
    use serde_json::json;

    fn new_session() -> Session {
        let mut s = Session::new(Workspace::none());
        s.execute("file.new", &json!({"units": "mm"})).unwrap();
        s
    }

    #[test]
    fn sets_and_keeps_the_tolerances_and_undoes() {
        let mut s = new_session();
        let out = s
            .execute("document.setTolerance", &json!({"angular": 1e-6}))
            .unwrap();
        assert_eq!(out["angular"], 1e-6);
        assert_eq!(out["linear"], 1e-6);
        assert_eq!(out["was"]["angular"], 1e-9);
        let doc = s.document().unwrap().history.current().tolerance;
        assert_eq!((doc.linear, doc.angular), (1e-6, 1e-6));
        let out = s
            .execute(
                "document.setTolerance",
                &json!({"linear": 1e-4, "loosest": 0.1}),
            )
            .unwrap();
        assert_eq!(out["angular"], 1e-6, "kept");
        assert_eq!(out["loosest"], 0.1);
        let out = s
            .execute("document.setTolerance", &json!({"loosest": null}))
            .unwrap();
        assert_eq!(out["loosest"], 0.1, "1000 times linear");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        let doc = s.document().unwrap().history.current().tolerance;
        assert_eq!((doc.linear, doc.angular), (1e-6, 1e-9));
    }

    #[test]
    fn refuses_what_is_out_of_range_and_changes_nothing() {
        let mut s = new_session();
        for bad in [
            json!({"angular": 0}),
            json!({"angular": -1e-6}),
            json!({"angular": 0.5}),
            json!({"angular": "x"}),
            json!({"linear": 0}),
            json!({"linear": 1e9}),
            json!({"linear": 1e-3, "loosest": 1e-4}),
            json!({"loosest": "far"}),
            json!({"loosest": -1}),
            json!({"color": 1}),
        ] {
            assert!(s.execute("document.setTolerance", &bad).is_err(), "{bad}");
        }
        let doc = s.document().unwrap().history.current().tolerance;
        assert_eq!((doc.linear, doc.angular, doc.loosest), (1e-6, 1e-9, None));
    }
}
